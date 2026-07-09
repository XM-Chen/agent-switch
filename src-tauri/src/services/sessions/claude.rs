//! Claude Code 会话 JSONL 只读扫描。
//!
//! 只读取 `~/.claude/projects/**/*.jsonl`，不删除、不移动、不写回。列表解析只看
//! 文件头部和尾部少量行，详情按需逐行读取单个会话文件。

use serde::Serialize;
use serde_json::Value;
use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const APP_TYPE: &str = "claude-code";
const HEAD_LINES: usize = 80;
const TAIL_LINES: usize = 80;
const TAIL_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone)]
pub struct SessionQuery {
    pub limit: usize,
    pub offset: usize,
    pub search: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionMeta {
    pub app_type: String,
    pub session_id: String,
    pub title: String,
    pub summary: Option<String>,
    pub project_dir: Option<String>,
    pub created_at_ms: Option<i64>,
    pub last_active_at_ms: Option<i64>,
    pub source_path: String,
    pub resume_command: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionListResponse {
    pub items: Vec<SessionMeta>,
    pub total: usize,
    pub limit: usize,
    pub offset: usize,
    pub scan_root: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionMessage {
    pub role: String,
    pub content: String,
    pub timestamp_ms: Option<i64>,
    pub raw_kind: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionMessagesResponse {
    pub source_path: String,
    pub messages: Vec<SessionMessage>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Default)]
struct MetaParts {
    session_id: Option<String>,
    project_dir: Option<String>,
    custom_title: Option<String>,
    first_user_title: Option<String>,
    summary: Option<String>,
    created_at_ms: Option<i64>,
    last_active_at_ms: Option<i64>,
    warnings: Vec<String>,
}

pub fn sessions_root() -> Result<PathBuf, String> {
    dirs::home_dir()
        .map(|h| h.join(".claude").join("projects"))
        .ok_or_else(|| "无法获取用户主目录".to_string())
}

pub fn scan_sessions(query: SessionQuery) -> Result<SessionListResponse, String> {
    scan_sessions_at(&sessions_root()?, query)
}

pub fn scan_sessions_at(root: &Path, query: SessionQuery) -> Result<SessionListResponse, String> {
    let scan_root = root.to_string_lossy().to_string();
    if !root.exists() {
        return Ok(SessionListResponse {
            items: Vec::new(),
            total: 0,
            limit: query.limit,
            offset: query.offset,
            scan_root,
            warnings: Vec::new(),
        });
    }

    let mut warnings = Vec::new();
    let mut files = Vec::new();
    collect_jsonl_files(root, &mut files, &mut warnings);

    let mut items = Vec::new();
    for path in files {
        match parse_session_meta(&path) {
            Ok(meta) => items.push(meta),
            Err(e) => warnings.push(format!("跳过 {}: {}", path.display(), e)),
        }
    }

    if let Some(search) = query
        .search
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let needle = search.to_lowercase();
        items.retain(|item| session_matches(item, &needle));
    }

    items.sort_by(|a, b| {
        let a_time = a.last_active_at_ms.or(a.created_at_ms).unwrap_or(0);
        let b_time = b.last_active_at_ms.or(b.created_at_ms).unwrap_or(0);
        b_time.cmp(&a_time)
    });

    let total = items.len();
    let paged = items
        .into_iter()
        .skip(query.offset)
        .take(query.limit)
        .collect();

    Ok(SessionListResponse {
        items: paged,
        total,
        limit: query.limit,
        offset: query.offset,
        scan_root,
        warnings,
    })
}

pub fn read_session_messages(source_path: &str) -> Result<SessionMessagesResponse, String> {
    read_session_messages_at(&sessions_root()?, source_path)
}

pub fn read_session_messages_at(
    root: &Path,
    source_path: &str,
) -> Result<SessionMessagesResponse, String> {
    let path = validate_source_path(root, source_path)?;
    let file = File::open(&path).map_err(|e| format!("读取 {} 失败: {}", path.display(), e))?;
    let mut reader = BufReader::new(file);
    let mut buf = Vec::new();
    let mut line_no = 0usize;
    let mut messages = Vec::new();
    let mut warnings = Vec::new();

    loop {
        buf.clear();
        let n = reader
            .read_until(b'\n', &mut buf)
            .map_err(|e| format!("读取 {} 失败: {}", path.display(), e))?;
        if n == 0 {
            break;
        }
        line_no += 1;
        let line = line_from_bytes(&buf);
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(&line) {
            Ok(value) => {
                if let Some(message) = parse_message_line(&value) {
                    messages.push(message);
                }
            }
            Err(e) => warnings.push(format!("第 {} 行 JSON 解析失败: {}", line_no, e)),
        }
    }

    Ok(SessionMessagesResponse {
        source_path: path.to_string_lossy().to_string(),
        messages,
        warnings,
    })
}

pub fn validate_source_path(root: &Path, source_path: &str) -> Result<PathBuf, String> {
    let root = root
        .canonicalize()
        .map_err(|e| format!("会话根目录不存在或不可访问: {}", e))?;
    let path = PathBuf::from(source_path)
        .canonicalize()
        .map_err(|e| format!("source_path 不存在或不可访问: {}", e))?;

    if !path.starts_with(&root) {
        return Err("source_path 不在 Claude Code 会话目录内".to_string());
    }
    if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
        return Err("source_path 必须是 .jsonl 文件".to_string());
    }
    if is_agent_jsonl(&path) {
        return Err("agent-*.jsonl 子代理会话不支持在主列表中打开".to_string());
    }
    if !path.is_file() {
        return Err("source_path 不是文件".to_string());
    }
    Ok(path)
}

fn collect_jsonl_files(root: &Path, out: &mut Vec<PathBuf>, warnings: &mut Vec<String>) {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) => {
            warnings.push(format!("读取目录 {} 失败: {}", root.display(), e));
            return;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                warnings.push(format!("读取目录项失败: {}", e));
                continue;
            }
        };
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(e) => {
                warnings.push(format!("读取 {} 类型失败: {}", path.display(), e));
                continue;
            }
        };
        if file_type.is_dir() {
            collect_jsonl_files(&path, out, warnings);
        } else if file_type.is_file()
            && path.extension().and_then(|s| s.to_str()) == Some("jsonl")
            && !is_agent_jsonl(&path)
        {
            out.push(path);
        }
    }
}

fn is_agent_jsonl(path: &Path) -> bool {
    path.file_name()
        .and_then(|s| s.to_str())
        .map(|name| name.starts_with("agent-") && name.ends_with(".jsonl"))
        .unwrap_or(false)
}

fn parse_session_meta(path: &Path) -> Result<SessionMeta, String> {
    let mut parts = MetaParts::default();
    let mut lines = read_first_lines(path, HEAD_LINES, &mut parts.warnings)?;
    let tail = read_tail_lines(path, TAIL_LINES, &mut parts.warnings)?;
    lines.extend(tail);

    for (idx, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(line) {
            Ok(value) => inspect_meta_line(&value, &mut parts),
            Err(e) => parts.warnings.push(format!(
                "{} 第 {} 行附近 JSON 解析失败: {}",
                path.display(),
                idx + 1,
                e
            )),
        }
    }

    let file_stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("session")
        .to_string();
    let session_id = parts.session_id.unwrap_or(file_stem);
    let project_title = parts
        .project_dir
        .as_deref()
        .and_then(project_basename)
        .or_else(|| {
            path.parent()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .map(clean_project_folder_name)
        });
    let title = parts
        .custom_title
        .or(parts.first_user_title)
        .or(project_title)
        .unwrap_or_else(|| compact_text(&session_id, 12));

    Ok(SessionMeta {
        app_type: APP_TYPE.to_string(),
        resume_command: Some(format!("claude --resume {}", session_id)),
        session_id,
        title,
        summary: parts.summary,
        project_dir: parts.project_dir,
        created_at_ms: parts.created_at_ms,
        last_active_at_ms: parts.last_active_at_ms,
        source_path: path.to_string_lossy().to_string(),
        warnings: parts.warnings,
    })
}

fn inspect_meta_line(value: &Value, parts: &mut MetaParts) {
    if parts.session_id.is_none() {
        parts.session_id = string_field(value, &["sessionId", "session_id"]).map(str::to_string);
    }
    if parts.project_dir.is_none() {
        parts.project_dir =
            string_field(value, &["cwd", "projectDir", "project_dir"]).map(str::to_string);
    }

    if let Some(ts) = timestamp_ms(value) {
        if parts.created_at_ms.is_none() {
            parts.created_at_ms = Some(ts);
        }
        parts.last_active_at_ms = Some(parts.last_active_at_ms.map_or(ts, |old| old.max(ts)));
    }

    let kind = value.get("type").and_then(Value::as_str);
    let subtype = value.get("subtype").and_then(Value::as_str);
    if kind == Some("custom-title") || subtype == Some("custom-title") {
        if let Some(title) = string_field(value, &["title", "summary", "content"]) {
            let title = compact_text(title, 120);
            if !title.is_empty() {
                parts.custom_title = Some(title);
            }
        }
    }

    if parts.summary.is_none() {
        if let Some(summary) = string_field(value, &["summary"]) {
            let summary = compact_text(summary, 500);
            if !summary.is_empty() {
                parts.summary = Some(summary);
            }
        }
    }

    if parts.first_user_title.is_none() {
        let message = value.get("message").unwrap_or(value);
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .or(kind)
            .unwrap_or_default();
        if role == "user" {
            if let Some(content) = message.get("content") {
                if !is_pure_tool_result_content(content) {
                    let title = compact_text(&render_content(content), 120);
                    if !title.is_empty() {
                        parts.first_user_title = Some(title);
                    }
                }
            }
        }
    }
}

fn read_first_lines(
    path: &Path,
    max_lines: usize,
    warnings: &mut Vec<String>,
) -> Result<Vec<String>, String> {
    let file = File::open(path).map_err(|e| format!("读取 {} 失败: {}", path.display(), e))?;
    let mut reader = BufReader::new(file);
    let mut lines = Vec::new();
    let mut buf = Vec::new();
    for _ in 0..max_lines {
        buf.clear();
        let n = reader
            .read_until(b'\n', &mut buf)
            .map_err(|e| format!("读取 {} 失败: {}", path.display(), e))?;
        if n == 0 {
            break;
        }
        lines.push(line_from_bytes(&buf));
    }
    if lines.is_empty() {
        warnings.push(format!("{} 为空会话文件", path.display()));
    }
    Ok(lines)
}

fn read_tail_lines(
    path: &Path,
    max_lines: usize,
    warnings: &mut Vec<String>,
) -> Result<Vec<String>, String> {
    let mut file = File::open(path).map_err(|e| format!("读取 {} 失败: {}", path.display(), e))?;
    let len = file
        .metadata()
        .map_err(|e| format!("读取 {} 元数据失败: {}", path.display(), e))?
        .len();
    let start = len.saturating_sub(TAIL_BYTES);
    file.seek(SeekFrom::Start(start))
        .map_err(|e| format!("读取 {} 尾部失败: {}", path.display(), e))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .map_err(|e| format!("读取 {} 尾部失败: {}", path.display(), e))?;
    let mut text = String::from_utf8_lossy(&buf).to_string();
    if start > 0 {
        text = text
            .find('\n')
            .map(|idx| text[idx + 1..].to_string())
            .unwrap_or_default();
    }
    let mut tail = VecDeque::new();
    for line in text.lines() {
        if tail.len() == max_lines {
            tail.pop_front();
        }
        tail.push_back(line.to_string());
    }
    if tail.is_empty() && len > 0 {
        warnings.push(format!("{} 尾部没有可读行", path.display()));
    }
    Ok(tail.into_iter().collect())
}

fn line_from_bytes(buf: &[u8]) -> String {
    String::from_utf8_lossy(buf)
        .trim_end_matches(['\r', '\n'])
        .to_string()
}

fn parse_message_line(value: &Value) -> Option<SessionMessage> {
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .map(str::to_string);
    let message = value.get("message").unwrap_or(value);
    let content = message.get("content").or_else(|| value.get("content"))?;
    let mut role = message
        .get("role")
        .and_then(Value::as_str)
        .or_else(|| value.get("role").and_then(Value::as_str))
        .or(kind.as_deref())
        .unwrap_or("message")
        .to_string();

    if role == "user" && is_pure_tool_result_content(content) {
        role = "tool".to_string();
    }

    let rendered = render_content(content);
    if rendered.trim().is_empty() {
        return None;
    }

    Some(SessionMessage {
        role,
        content: rendered,
        timestamp_ms: timestamp_ms(value).or_else(|| timestamp_ms(message)),
        raw_kind: kind,
    })
}

fn render_content(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .map(render_content_part)
            .filter(|s| !s.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
        Value::Object(_) => render_content_part(value),
        _ => value.to_string(),
    }
}

fn render_content_part(value: &Value) -> String {
    let Some(obj) = value.as_object() else {
        return render_content(value);
    };
    match obj.get("type").and_then(Value::as_str) {
        Some("text") => obj
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        Some("tool_use") => {
            let name = obj.get("name").and_then(Value::as_str).unwrap_or("unknown");
            match obj.get("input") {
                Some(input) if !input.is_null() => {
                    format!("工具调用: {}\n{}", name, json_pretty(input))
                }
                _ => format!("工具调用: {}", name),
            }
        }
        Some("tool_result") => match obj.get("content") {
            Some(content) => format!("工具结果:\n{}", render_content(content)),
            None => "工具结果".to_string(),
        },
        _ => {
            if let Some(text) = obj.get("text").and_then(Value::as_str) {
                text.to_string()
            } else if let Some(content) = obj.get("content") {
                render_content(content)
            } else {
                json_pretty(value)
            }
        }
    }
}

fn is_pure_tool_result_content(value: &Value) -> bool {
    match value {
        Value::Array(items) if !items.is_empty() => items.iter().all(is_tool_result),
        Value::Object(_) => is_tool_result(value),
        _ => false,
    }
}

fn is_tool_result(value: &Value) -> bool {
    value
        .as_object()
        .and_then(|obj| obj.get("type"))
        .and_then(Value::as_str)
        == Some("tool_result")
}

fn string_field<'a>(value: &'a Value, names: &[&str]) -> Option<&'a str> {
    names
        .iter()
        .find_map(|name| value.get(*name).and_then(Value::as_str))
}

fn session_matches(item: &SessionMeta, needle: &str) -> bool {
    [
        Some(item.title.as_str()),
        item.summary.as_deref(),
        item.project_dir.as_deref(),
        Some(item.session_id.as_str()),
        Some(item.source_path.as_str()),
    ]
    .into_iter()
    .flatten()
    .any(|value| value.to_lowercase().contains(needle))
}

fn project_basename(project_dir: &str) -> Option<String> {
    Path::new(project_dir)
        .file_name()
        .and_then(|s| s.to_str())
        .map(str::to_string)
}

fn clean_project_folder_name(name: &str) -> String {
    name.trim_matches('-').replace('-', "/")
}

fn compact_text(text: &str, max_chars: usize) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= max_chars {
        return one_line;
    }
    let mut out = one_line.chars().take(max_chars).collect::<String>();
    out.push('…');
    out
}

fn json_pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn timestamp_ms(value: &Value) -> Option<i64> {
    value
        .get("timestamp_ms")
        .and_then(Value::as_i64)
        .or_else(|| value.get("timestampMs").and_then(Value::as_i64))
        .or_else(|| value.get("timestamp").and_then(parse_timestamp_value))
        .or_else(|| value.get("createdAt").and_then(parse_timestamp_value))
}

fn parse_timestamp_value(value: &Value) -> Option<i64> {
    if let Some(n) = value.as_i64() {
        return Some(n);
    }
    let s = value.as_str()?.trim();
    s.parse::<i64>().ok().or_else(|| parse_rfc3339_ms(s))
}

fn parse_rfc3339_ms(s: &str) -> Option<i64> {
    if s.len() < 19 {
        return None;
    }
    let year = s.get(0..4)?.parse::<i32>().ok()?;
    let month = s.get(5..7)?.parse::<u32>().ok()?;
    let day = s.get(8..10)?.parse::<u32>().ok()?;
    let hour = s.get(11..13)?.parse::<i64>().ok()?;
    let minute = s.get(14..16)?.parse::<i64>().ok()?;
    let second = s.get(17..19)?.parse::<i64>().ok()?;

    let mut idx = 19;
    let mut millis = 0i64;
    if s.as_bytes().get(idx) == Some(&b'.') {
        idx += 1;
        let frac_start = idx;
        while idx < s.len() && s.as_bytes()[idx].is_ascii_digit() {
            idx += 1;
        }
        let frac = s.get(frac_start..idx).unwrap_or_default();
        let mut ms_digits = frac.chars().take(3).collect::<String>();
        while ms_digits.len() < 3 {
            ms_digits.push('0');
        }
        millis = ms_digits.parse::<i64>().ok()?;
    }

    let offset_seconds = match s.get(idx..idx + 1) {
        Some("Z") | None | Some("") => 0,
        Some("+") | Some("-") => {
            let sign = if s.get(idx..idx + 1) == Some("+") {
                1
            } else {
                -1
            };
            let hh = s.get(idx + 1..idx + 3)?.parse::<i64>().ok()?;
            let mm = s.get(idx + 4..idx + 6)?.parse::<i64>().ok()?;
            sign * (hh * 3600 + mm * 60)
        }
        _ => 0,
    };

    let days = days_from_civil(year, month, day)?;
    Some((days * 86_400 + hour * 3600 + minute * 60 + second - offset_seconds) * 1000 + millis)
}

fn days_from_civil(year: i32, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let year = year - i32::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let month = month as i32;
    let day = day as i32;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146097 + doe - 719468) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_dir(tag: &str) -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "as-sessions-test-{}-{}-{}",
            tag,
            std::process::id(),
            n
        ));
        std::fs::create_dir_all(&dir).expect("创建临时目录失败");
        dir
    }

    fn write_session(path: &Path, lines: &[&str]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, format!("{}\n", lines.join("\n"))).unwrap();
    }

    #[test]
    fn missing_root_returns_empty_list() {
        let root =
            std::env::temp_dir().join(format!("as-sessions-missing-{}-nope", std::process::id()));
        let resp = scan_sessions_at(
            &root,
            SessionQuery {
                limit: 50,
                offset: 0,
                search: None,
            },
        )
        .unwrap();
        assert_eq!(resp.total, 0);
        assert_eq!(resp.items.len(), 0);
        assert_eq!(resp.scan_root, root.to_string_lossy());
        assert!(!root.exists(), "只读扫描不应创建目录");
    }

    #[test]
    fn scans_jsonl_skips_agent_and_uses_title_priority() {
        let root = unique_dir("scan");
        let session = root.join("proj").join("s1.jsonl");
        write_session(
            &session,
            &[
                r#"{"type":"user","sessionId":"sid-1","cwd":"/tmp/my-project","timestamp":"1970-01-01T00:00:01.000Z","message":{"role":"user","content":"first user title"}}"#,
                r#"{"type":"summary","summary":"session summary","timestamp":"1970-01-01T00:00:02Z"}"#,
                r#"{"type":"system","subtype":"custom-title","title":"Custom Title","timestamp":"1970-01-01T00:00:03Z"}"#,
            ],
        );
        write_session(
            &root.join("proj").join("agent-child.jsonl"),
            &[r#"{"type":"user","message":{"role":"user","content":"skip"}}"#],
        );

        let resp = scan_sessions_at(
            &root,
            SessionQuery {
                limit: 50,
                offset: 0,
                search: None,
            },
        )
        .unwrap();
        assert_eq!(resp.total, 1);
        let item = &resp.items[0];
        assert_eq!(item.session_id, "sid-1");
        assert_eq!(item.title, "Custom Title");
        assert_eq!(item.summary.as_deref(), Some("session summary"));
        assert_eq!(item.last_active_at_ms, Some(3000));
        assert_eq!(
            item.resume_command.as_deref(),
            Some("claude --resume sid-1")
        );
    }

    #[test]
    fn bad_json_line_is_warning_not_fatal() {
        let root = unique_dir("badline");
        let session = root.join("proj").join("s1.jsonl");
        write_session(
            &session,
            &[
                "not-json",
                r#"{"type":"user","sessionId":"sid","message":{"role":"user","content":"hello"}}"#,
            ],
        );
        let resp = scan_sessions_at(
            &root,
            SessionQuery {
                limit: 50,
                offset: 0,
                search: None,
            },
        )
        .unwrap();
        assert_eq!(resp.total, 1);
        assert!(!resp.items[0].warnings.is_empty());

        let detail = read_session_messages_at(&root, &session.to_string_lossy()).unwrap();
        assert_eq!(detail.messages.len(), 1);
        assert_eq!(detail.messages[0].content, "hello");
        assert!(!detail.warnings.is_empty());
    }

    #[test]
    fn search_and_pagination_apply_after_filtering() {
        let root = unique_dir("search");
        write_session(
            &root.join("p").join("a.jsonl"),
            &[
                r#"{"type":"user","sessionId":"a","timestamp_ms":1,"message":{"role":"user","content":"alpha"}}"#,
            ],
        );
        write_session(
            &root.join("p").join("b.jsonl"),
            &[
                r#"{"type":"user","sessionId":"b","timestamp_ms":2,"message":{"role":"user","content":"beta"}}"#,
            ],
        );
        let resp = scan_sessions_at(
            &root,
            SessionQuery {
                limit: 1,
                offset: 0,
                search: Some("a".to_string()),
            },
        )
        .unwrap();
        assert_eq!(resp.total, 2);
        assert_eq!(resp.items.len(), 1);
        assert_eq!(resp.items[0].session_id, "b", "最近活跃倒序");
    }

    #[test]
    fn validate_source_path_rejects_outside_and_non_jsonl() {
        let root = unique_dir("path");
        let inside = root.join("p").join("s.jsonl");
        write_session(
            &inside,
            &[r#"{"type":"user","message":{"role":"user","content":"ok"}}"#],
        );
        let outside_dir = unique_dir("outside");
        let outside = outside_dir.join("s.jsonl");
        write_session(&outside, &[r#"{}"#]);
        let txt = root.join("p").join("s.txt");
        std::fs::write(&txt, "x").unwrap();

        assert!(validate_source_path(&root, &inside.to_string_lossy()).is_ok());
        assert!(validate_source_path(&root, &outside.to_string_lossy())
            .unwrap_err()
            .contains("不在"));
        assert!(validate_source_path(&root, &txt.to_string_lossy())
            .unwrap_err()
            .contains(".jsonl"));
    }

    #[test]
    fn details_parse_roles_and_tool_results() {
        let root = unique_dir("messages");
        let session = root.join("p").join("s.jsonl");
        write_session(
            &session,
            &[
                r#"{"type":"user","timestamp":"1970-01-01T00:00:01Z","message":{"role":"user","content":"hello"}}"#,
                r#"{"type":"assistant","timestamp":"1970-01-01T00:00:02Z","message":{"role":"assistant","content":[{"type":"text","text":"hi"},{"type":"tool_use","name":"Read","input":{"file_path":"a"}}]}}"#,
                r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"file content"}]}}"#,
                r#"{"type":"summary","summary":"metadata only"}"#,
            ],
        );

        let detail = read_session_messages_at(&root, &session.to_string_lossy()).unwrap();
        assert_eq!(detail.messages.len(), 3);
        assert_eq!(detail.messages[0].role, "user");
        assert_eq!(detail.messages[0].timestamp_ms, Some(1000));
        assert_eq!(detail.messages[1].role, "assistant");
        assert!(detail.messages[1].content.contains("工具调用: Read"));
        assert_eq!(detail.messages[2].role, "tool");
        assert!(detail.messages[2].content.contains("file content"));
    }
}
