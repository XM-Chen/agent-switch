/// SSE 行解码器。
///
/// 流式响应的 chunk 边界与 SSE 行边界不对齐：一个 chunk 可能含半行，也可能跨多行。
/// `SseLineDecoder` 累积字节，按 `\n` 切出完整行，供 `translate_stream_line` 逐行转换后再下发。
///
/// 仅用于跨协议流式转换；同协议流式直通，不需要本解码器。
pub struct SseLineDecoder {
    /// 跨 chunk 累积的未完成行字节。
    buf: Vec<u8>,
}

/// 流式字节流类型别名（与 `StreamGuard::BufferedResult.remaining_stream` 一致）。
pub type ByteStream = std::pin::Pin<
    Box<
        dyn futures::stream::Stream<
                Item = Result<bytes::Bytes, Box<dyn std::error::Error + Send + Sync>>,
            > + Send,
    >,
>;

/// 把上游流式响应逐行经转换器翻译后再下发。
///
/// - 同协议流式不应调用本函数（直接透传 `remaining_stream` 即可）。
/// - 跨协议流式：每个 chunk 喂入 `SseLineDecoder` 切出完整行，每行调
///   `translator.translate_stream_line`，转换结果重新拼成字节下发。
/// - 流结束时 flush 解码器残余行。
/// - 转换错误：合成一条错误终端事件后结束流（不让客户端永远挂起）。
pub fn translate_stream(
    inner: ByteStream,
    translator: std::sync::Arc<dyn crate::services::translator::Translator>,
    model: String,
    inbound_protocol: &str,
) -> ByteStream {
    use futures::stream::StreamExt;

    let state = TransStreamState {
        inner,
        decoder: SseLineDecoder::new(),
        translator,
        ctx: crate::services::translator::StreamContext::new("", &model, 0),
        inbound_protocol: inbound_protocol.to_string(),
        errored: false,
    };

    Box::pin(futures::stream::unfold(state, |mut state| async move {
        if state.errored {
            return None;
        }
        loop {
            match state.inner.next().await {
                Some(Ok(chunk)) => {
                    let lines = state.decoder.push(&chunk);
                    if lines.is_empty() {
                        continue;
                    }
                    let mut out = String::new();
                    for line in &lines {
                        match state.translator.translate_stream_line(line, &mut state.ctx) {
                            Ok(t) => out.push_str(&t),
                            Err(e) => {
                                let evt = crate::services::translator::helpers::build_error_event(
                                    &format!("流式转换错误: {}", e),
                                    &state.inbound_protocol,
                                );
                                state.errored = true;
                                return Some((Ok(bytes::Bytes::from(evt)), state));
                            }
                        }
                    }
                    if !out.is_empty() {
                        return Some((Ok(bytes::Bytes::from(out)), state));
                    }
                }
                Some(Err(e)) => {
                    let evt = crate::services::translator::helpers::build_error_event(
                        &format!("上游流错误: {}", e),
                        &state.inbound_protocol,
                    );
                    return Some((Ok(bytes::Bytes::from(evt)), state));
                }
                None => {
                    // 流结束：flush 残余行
                    if let Some(last) = state.decoder.flush() {
                        match state
                            .translator
                            .translate_stream_line(&last, &mut state.ctx)
                        {
                            Ok(t) => {
                                return Some((Ok(bytes::Bytes::from(t)), state));
                            }
                            Err(_) => return None,
                        }
                    }
                    return None;
                }
            }
        }
    }))
}

struct TransStreamState {
    inner: ByteStream,
    decoder: SseLineDecoder,
    translator: std::sync::Arc<dyn crate::services::translator::Translator>,
    ctx: crate::services::translator::StreamContext,
    inbound_protocol: String,
    errored: bool,
}

impl SseLineDecoder {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// 喂入一个 chunk 的字节，返回本次切出的完整行（不含行尾 `\n`）。
    ///
    /// 末尾不带 `\n` 的残余字节保留在内部缓冲，等下一个 chunk 补齐。
    pub fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(chunk);
        let mut lines = Vec::new();
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let line_bytes = self.buf[..pos].to_vec();
            self.buf = self.buf[pos + 1..].to_vec();
            // SSE 行按字符串处理；丢弃尾部的 \r（CRLF 行尾）
            let mut line = String::from_utf8_lossy(&line_bytes).into_owned();
            if line.ends_with('\r') {
                line.pop();
            }
            lines.push(line);
        }
        lines
    }

    /// 流结束时 flush 残余缓冲（若最后一段无 `\n` 也应作为一行输出）。
    pub fn flush(&mut self) -> Option<String> {
        if self.buf.is_empty() {
            return None;
        }
        let line_bytes = std::mem::take(&mut self.buf);
        let mut line = String::from_utf8_lossy(&line_bytes).into_owned();
        if line.ends_with('\r') {
            line.pop();
        }
        Some(line)
    }
}

impl Default for SseLineDecoder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_complete_lines_in_one_chunk() {
        let mut d = SseLineDecoder::new();
        let lines = d.push(b"data: {\"a\":1}\n\ndata: {\"b\":2}\n\n");
        assert_eq!(lines, vec!["data: {\"a\":1}", "", "data: {\"b\":2}", ""]);
    }

    #[test]
    fn buffers_partial_line_across_chunks() {
        let mut d = SseLineDecoder::new();
        let first = d.push(b"data: {\"part");
        assert!(first.is_empty(), "半行不应产出");
        let second = d.push(b"ial\":1}\n");
        assert_eq!(second, vec!["data: {\"partial\":1}"]);
    }

    #[test]
    fn flushes_trailing_line_without_newline() {
        let mut d = SseLineDecoder::new();
        d.push(b"data: {\"x\":1}\n");
        d.push(b"data: {\"y\":2}"); // 无尾随 \n
        let flushed = d.flush();
        assert_eq!(flushed, Some("data: {\"y\":2}".to_string()));
    }

    #[test]
    fn strips_crlf_carriage_return() {
        let mut d = SseLineDecoder::new();
        let lines = d.push(b"data: {\"a\":1}\r\n\r\n");
        assert_eq!(lines, vec!["data: {\"a\":1}", ""]);
    }

    // 修复 push(&mut self, b"literal") 的歧义：提供一个显式辅助避免与上面 push 函数冲突。
    // （上面 push 辅助函数仅服务于第二个测试；后续测试直接用实例方法。）
    #[test]
    fn handles_empty_chunk() {
        let mut d = SseLineDecoder::new();
        assert!(d.push(b"").is_empty());
        assert!(d.flush().is_none());
    }
}
