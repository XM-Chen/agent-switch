//! Skills 网络下载与压缩包安全解包层（cc-skills 阶段 C）。
//!
//! 职责：
//! - 从 GitHub 拉取 repo tarball（异步 reqwest），仅在用户显式触发时联网。
//! - 安全解包 tar.gz / zip：拒绝路径穿越（`..`、绝对路径）、拒绝符号链接/硬链接。
//! - 在解包后的目录树中定位包含 `SKILL.md` 的 skill 目录。
//!
//! 所有解包都落到调用方给定的临时根目录内；本模块不写 SSOT，也不碰 live 投影。

use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};

use super::ENTRY_FILE;

/// GitHub 来源描述：owner/name + 可选分支与子目录。
#[derive(Debug, Clone)]
pub struct GithubSource {
    pub owner: String,
    pub name: String,
    pub branch: Option<String>,
    pub subdir: Option<String>,
}

impl GithubSource {
    /// 解析 `owner/name` 形态的 repo 字符串。
    pub fn parse_repo(repo: &str) -> Result<(String, String), String> {
        let parts: Vec<&str> = repo.split('/').collect();
        if parts.len() != 2 || parts.iter().any(|p| p.trim().is_empty()) {
            return Err("repo 必须是 owner/name 形态".to_string());
        }
        Ok((parts[0].trim().to_string(), parts[1].trim().to_string()))
    }

    pub fn repo_url(&self) -> String {
        format!("https://github.com/{}/{}", self.owner, self.name)
    }

    fn tarball_ref(&self) -> String {
        self.branch.clone().unwrap_or_else(|| "HEAD".to_string())
    }

    fn tarball_url(&self) -> String {
        format!(
            "https://api.github.com/repos/{}/{}/tarball/{}",
            self.owner,
            self.name,
            self.tarball_ref()
        )
    }
}

/// 构造带 User-Agent 的 HTTP 客户端；GitHub API 强制要求 UA。
pub fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent("agent-switch")
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))
}

/// 异步下载 GitHub repo tarball 字节。token 可选（匿名 60 次/小时限速）。
pub async fn download_repo_tarball(
    client: &reqwest::Client,
    source: &GithubSource,
    token: Option<&str>,
) -> Result<Vec<u8>, String> {
    let mut req = client
        .get(source.tarball_url())
        .header("Accept", "application/vnd.github+json");
    if let Some(token) = token {
        if !token.trim().is_empty() {
            req = req.header("Authorization", format!("Bearer {}", token.trim()));
        }
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("下载 GitHub tarball 失败: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        if status.as_u16() == 404 {
            return Err(format!(
                "GitHub 未找到 repo 或分支: {}/{}",
                source.owner, source.name
            ));
        }
        if status.as_u16() == 403 {
            return Err("GitHub API 访问受限（可能触发匿名限速），稍后重试或配置 token".to_string());
        }
        return Err(format!("GitHub 返回错误状态: {}", status));
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("读取 tarball 响应失败: {}", e))?;
    Ok(bytes.to_vec())
}

/// 校验单个归档条目的相对路径是安全的：非空、无 `..`、非绝对路径。
fn safe_relative(path: &Path) -> Result<PathBuf, String> {
    let mut out = PathBuf::new();
    let mut has_component = false;
    for comp in path.components() {
        match comp {
            Component::Normal(seg) => {
                has_component = true;
                out.push(seg);
            }
            Component::CurDir => {}
            Component::ParentDir => {
                return Err("归档包含 .. 路径，拒绝解包".to_string());
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err("归档包含绝对路径，拒绝解包".to_string());
            }
        }
    }
    if !has_component {
        return Err("归档条目路径为空".to_string());
    }
    Ok(out)
}

/// 安全解包 tar.gz 到 `dest_root`；返回解出的单一顶层目录（GitHub tarball 形态）。
///
/// 拒绝符号链接/硬链接与路径穿越。`dest_root` 必须已存在且为空可写。
pub fn unpack_tarball_to(bytes: &[u8], dest_root: &Path) -> Result<PathBuf, String> {
    use tar::EntryType;

    let gz = flate2::read::GzDecoder::new(bytes);
    let mut archive = tar::Archive::new(gz);
    let entries = archive
        .entries()
        .map_err(|e| format!("读取 tar 条目失败: {}", e))?;

    let mut top_dirs: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    for entry in entries {
        let mut entry = entry.map_err(|e| format!("读取 tar 条目失败: {}", e))?;
        let entry_type = entry.header().entry_type();
        match entry_type {
            EntryType::Symlink | EntryType::Link => {
                return Err("tar 归档包含链接条目，拒绝解包".to_string());
            }
            EntryType::Regular | EntryType::Directory | EntryType::GNULongName => {}
            other => {
                return Err(format!("tar 归档包含不支持的条目类型: {:?}", other));
            }
        }

        let path = entry
            .path()
            .map_err(|e| format!("解析 tar 条目路径失败: {}", e))?
            .into_owned();
        let rel = safe_relative(&path)?;
        if let Some(Component::Normal(seg)) = rel.components().next() {
            if let Some(s) = seg.to_str() {
                top_dirs.insert(s.to_string());
            }
        }

        let dest = dest_root.join(&rel);
        ensure_within(dest_root, &dest)?;

        if entry_type == EntryType::Directory {
            std::fs::create_dir_all(&dest)
                .map_err(|e| format!("创建解包目录失败 {}: {}", dest.display(), e))?;
            continue;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("创建解包父目录失败: {}", e))?;
        }
        let mut buf = Vec::new();
        entry
            .read_to_end(&mut buf)
            .map_err(|e| format!("读取 tar 条目内容失败: {}", e))?;
        std::fs::write(&dest, &buf)
            .map_err(|e| format!("写入解包文件失败 {}: {}", dest.display(), e))?;
    }

    resolve_single_top_dir(dest_root, &top_dirs)
}

/// 安全解包 zip 到 `dest_root`；返回解包根（zip 内容平铺，不强制单顶层目录）。
pub fn unpack_zip_to(bytes: &[u8], dest_root: &Path) -> Result<PathBuf, String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| format!("打开 zip 失败: {}", e))?;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("读取 zip 条目失败: {}", e))?;

        // enclosed_name 对路径穿越返回 None。
        let rel = file
            .enclosed_name()
            .ok_or_else(|| "zip 条目路径不安全，拒绝解包".to_string())?;
        let rel = safe_relative(&rel)?;

        // 拒绝符号链接（unix mode 高位 0o120000）。
        if let Some(mode) = file.unix_mode() {
            if mode & 0o170000 == 0o120000 {
                return Err("zip 归档包含符号链接，拒绝解包".to_string());
            }
        }

        let dest = dest_root.join(&rel);
        ensure_within(dest_root, &dest)?;

        if file.is_dir() {
            std::fs::create_dir_all(&dest)
                .map_err(|e| format!("创建解包目录失败 {}: {}", dest.display(), e))?;
            continue;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("创建解包父目录失败: {}", e))?;
        }
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)
            .map_err(|e| format!("读取 zip 条目内容失败: {}", e))?;
        std::fs::write(&dest, &buf)
            .map_err(|e| format!("写入解包文件失败 {}: {}", dest.display(), e))?;
    }

    Ok(dest_root.to_path_buf())
}

/// 在解包结果中定位 skill 目录：优先 subdir，否则要求根含 `SKILL.md`。
pub fn locate_skill_dir(base: &Path, subdir: Option<&str>) -> Result<PathBuf, String> {
    let target = match subdir {
        Some(sub) if !sub.trim().is_empty() => {
            let mut dir = base.to_path_buf();
            for comp in Path::new(sub.trim()).components() {
                match comp {
                    Component::Normal(seg) => dir.push(seg),
                    Component::CurDir => {}
                    _ => return Err("subdir 含非法路径分量".to_string()),
                }
            }
            ensure_within(base, &dir)?;
            dir
        }
        _ => base.to_path_buf(),
    };

    if !target.is_dir() {
        return Err(format!("定位 skill 目录失败: {} 不存在", target.display()));
    }
    if !target.join(ENTRY_FILE).is_file() {
        return Err(format!("skill 目录缺少 {}", ENTRY_FILE));
    }
    Ok(target)
}

/// GitHub tarball 解出单一顶层目录时返回该目录，否则返回解包根。
fn resolve_single_top_dir(
    dest_root: &Path,
    top_dirs: &std::collections::BTreeSet<String>,
) -> Result<PathBuf, String> {
    if top_dirs.len() == 1 {
        let only = top_dirs.iter().next().unwrap();
        let candidate = dest_root.join(only);
        if candidate.is_dir() {
            return Ok(candidate);
        }
    }
    Ok(dest_root.to_path_buf())
}

/// 确认 `child` 位于 `root` 之下（词法判断，解包目标尚未存在无法 canonicalize）。
fn ensure_within(root: &Path, child: &Path) -> Result<(), String> {
    if child.starts_with(root) {
        Ok(())
    } else {
        Err(format!(
            "解包路径越界: {} 不在 {} 下",
            child.display(),
            root.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn tmp(tag: &str) -> PathBuf {
        static C: AtomicU64 = AtomicU64::new(0);
        let n = C.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "as-skills-dl-{}-{}-{}",
            tag,
            std::process::id(),
            n
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_targz(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut tar_buf = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_buf);
            for (name, content) in entries {
                let mut header = tar::Header::new_gnu();
                header.set_size(content.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                builder
                    .append_data(&mut header, name, &content[..])
                    .unwrap();
            }
            builder.finish().unwrap();
        }
        let mut gz = Vec::new();
        {
            use std::io::Write;
            let mut enc = flate2::write::GzEncoder::new(&mut gz, flate2::Compression::default());
            enc.write_all(&tar_buf).unwrap();
            enc.finish().unwrap();
        }
        gz
    }

    #[test]
    fn parse_repo_accepts_owner_name() {
        assert_eq!(
            GithubSource::parse_repo("owner/name").unwrap(),
            ("owner".to_string(), "name".to_string())
        );
        assert!(GithubSource::parse_repo("bad").is_err());
        assert!(GithubSource::parse_repo("a/b/c").is_err());
    }

    #[test]
    fn safe_relative_rejects_traversal_and_absolute() {
        assert!(safe_relative(Path::new("../evil")).is_err());
        assert!(safe_relative(Path::new("a/../../b")).is_err());
        assert!(safe_relative(Path::new("ok/sub/file.txt")).is_ok());
    }

    #[test]
    fn unpack_tarball_extracts_single_top_dir() {
        let gz = make_targz(&[
            ("repo-sha/SKILL.md", b"# demo\n"),
            ("repo-sha/scripts/run.sh", b"echo hi\n"),
        ]);
        let dest = tmp("tar-ok");
        let top = unpack_tarball_to(&gz, &dest).unwrap();
        assert!(top.join("SKILL.md").is_file());
        assert!(top.join("scripts/run.sh").is_file());
    }

    #[test]
    fn locate_skill_dir_uses_subdir() {
        let gz = make_targz(&[
            ("repo-sha/skills/foo/SKILL.md", b"# foo\n"),
            ("repo-sha/README.md", b"root\n"),
        ]);
        let dest = tmp("tar-subdir");
        let top = unpack_tarball_to(&gz, &dest).unwrap();
        let skill = locate_skill_dir(&top, Some("skills/foo")).unwrap();
        assert!(skill.join("SKILL.md").is_file());
        assert!(locate_skill_dir(&top, None).is_err());
    }

    #[test]
    fn locate_skill_dir_rejects_subdir_traversal() {
        let dest = tmp("subdir-traversal");
        std::fs::write(dest.join("SKILL.md"), "# x").unwrap();
        assert!(locate_skill_dir(&dest, Some("../../etc")).is_err());
    }
}
