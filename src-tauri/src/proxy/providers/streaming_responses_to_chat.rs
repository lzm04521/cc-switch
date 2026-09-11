//! Chat Completions SSE 转换模块（Responses 上游 → Chat 客户端）
//!
//! `/chat/completions` 入口 × Responses 型上游（2026-09-11）：上游吐 OpenAI
//! Responses API 的 named events 事件流，本地 Chat 协议客户端只认 delta chunk
//! 模型，本模块做流式协议转换。
//!
//! 与 `streaming_codex_chat.rs`（Chat 上游 → Responses 客户端）互为镜像；
//! 事件解析骨架参照 `streaming_responses.rs`（named events 生命周期模型）。
//!
//! 事件映射：
//! - response.created                       → 首个 chunk（delta.role=assistant）
//! - response.output_text.delta             → delta.content
//! - response.reasoning_summary_text.delta  → delta.reasoning_content
//!   （兼容 BigModel 私有 response.reasoning_text.delta，真机验收后收紧）
//! - response.output_item.added(function_call) → delta.tool_calls[index] 首帧
//! - response.function_call_arguments.delta → delta.tool_calls[index].arguments 增量
//! - response.completed / failed / incomplete → finish chunk + usage chunk + [DONE]

use bytes::Bytes;
use futures::stream::{Stream, StreamExt};
use serde_json::{json, Value};
use std::collections::HashMap;

use crate::proxy::sse::{strip_sse_field, take_sse_block};
use serde_json::Map as JsonMap;

fn chat_sse_chunk(payload: &Value) -> Bytes {
    Bytes::from(format!(
        "data: {}\n\n",
        serde_json::to_string(payload).unwrap_or_default()
    ))
}

fn done_marker() -> Bytes {
    Bytes::from_static("data: [DONE]\n\n".as_bytes())
}

#[derive(Debug, Default)]
struct ResponsesToChatState {
    /// 已产生 chunk（response.created 已发首个 role chunk）
    response_started: bool,
    completed: bool,
    response_id: String,
    model: String,
    created_at: u64,
    next_tool_index: u32,
    /// Responses item 的 output_index → chat tool_calls 的 index
    tool_index_by_output: HashMap<u64, u32>,
    /// output_index → function_call item（done 时校验用）
    pending_tools: HashMap<u64, Value>,
    /// 本回合因缺 name 被丢弃的工具调用数（finalize 防线）
    dropped_tool_calls: usize,
    finish_reason: Option<String>,
    usage: Option<Value>,
}

impl ResponsesToChatState {
    fn base_chunk(&self) -> Value {
        json!({
            "id": if self.response_id.is_empty() {
                "chatcmpl-ccswitch".to_string()
            } else {
                self.response_id.clone()
            },
            "object": "chat.completion.chunk",
            "created": self.created_at,
            "model": self.model.clone(),
        })
    }

    fn choice_chunk(&self, delta: Value, finish_reason: Option<&str>) -> Bytes {
        let mut payload = self.base_chunk();
        payload["choices"] = json!([{
            "index": 0,
            "delta": delta,
            "finish_reason": finish_reason,
        }]);
        chat_sse_chunk(&payload)
    }

    /// response.created：记录 id/model/created，发首个 role chunk
    fn handle_created(&mut self, data: &Value) -> Vec<Bytes> {
        let response = data.get("response").unwrap_or(data);
        self.response_id = response
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("chatcmpl-ccswitch")
            .to_string();
        self.model = response
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        self.created_at = response
            .get("created_at")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        self.response_started = true;
        vec![self.choice_chunk(
            json!({ "role": "assistant", "content": "" }),
            None,
        )]
    }

    /// response.output_item.added：function_call → tool_calls 首帧；其余占位
    fn handle_item_added(&mut self, data: &Value) -> Vec<Bytes> {
        let Some(item) = data.get("item") else {
            return Vec::new();
        };
        let output_index = data.get("output_index").and_then(Value::as_u64).unwrap_or(0);
        if item.get("type").and_then(Value::as_str) != Some("function_call") {
            return Vec::new();
        }
        let chat_index = self.next_tool_index;
        self.next_tool_index += 1;
        self.tool_index_by_output.insert(output_index, chat_index);
        self.pending_tools.insert(output_index, item.clone());

        let call_id = item
            .get("call_id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .unwrap_or("call_0");
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        vec![self.choice_chunk(
            json!({
                "tool_calls": [{
                    "index": chat_index,
                    "id": call_id,
                    "type": "function",
                    "function": { "name": name, "arguments": "" }
                }]
            }),
            None,
        )]
    }

    /// response.function_call_arguments.delta → arguments 增量
    fn handle_tool_arguments_delta(&mut self, data: &Value) -> Vec<Bytes> {
        let output_index = data.get("output_index").and_then(Value::as_u64).unwrap_or(0);
        let Some(chat_index) = self.tool_index_by_output.get(&output_index).copied() else {
            return Vec::new();
        };
        let delta = data.get("delta").and_then(Value::as_str).unwrap_or("");
        if delta.is_empty() {
            return Vec::new();
        }
        vec![self.choice_chunk(
            json!({
                "tool_calls": [{
                    "index": chat_index,
                    "function": { "arguments": delta }
                }]
            }),
            None,
        )]
    }

    /// response.output_item.done(function_call)：校验 name（缺 name 计 dropped）
    fn handle_item_done(&mut self, data: &Value) -> Vec<Bytes> {
        let Some(item) = data.get("item") else {
            return Vec::new();
        };
        if item.get("type").and_then(Value::as_str) != Some("function_call") {
            return Vec::new();
        }
        let output_index = data.get("output_index").and_then(Value::as_u64).unwrap_or(0);
        self.pending_tools.remove(&output_index);
        let has_name = item
            .get("name")
            .and_then(Value::as_str)
            .map(|name| !name.trim().is_empty())
            .unwrap_or(false);
        if !has_name {
            self.dropped_tool_calls += 1;
            log::debug!(
                "[ResponsesToChat] function_call item 缺 name，丢弃（output_index={output_index}）"
            );
        }
        Vec::new()
    }

    /// finish/usage/[DONE] 收尾。完成事件缺 finish 信号时按已知状态推断。
    fn finalize_chunks(&mut self, finish_reason: &str) -> Vec<Bytes> {
        if self.completed {
            return Vec::new();
        }
        self.completed = true;
        self.ensure_started();

        // 工具调用丢弃防线（镜像非流式分支与 streaming_codex_chat 的语义）：
        // 本应完成的回合里唯一工具调用被丢弃时如实报错，不谎报成功。
        if finish_reason != "length"
            && self.dropped_tool_calls > 0
            && self.next_tool_index as usize == self.dropped_tool_calls
        {
            let payload = json!({
                "error": {
                    "message": format!(
                        "Upstream returned {} function_call item(s) without a name, \
                         leaving no usable tool call in this turn",
                        self.dropped_tool_calls
                    ),
                    "type": "upstream_error",
                    "code": "tool_call_dropped"
                }
            });
            return vec![chat_sse_chunk(&payload), done_marker()];
        }

        let mut chunks = vec![self.choice_chunk(json!({}), Some(finish_reason))];
        if let Some(usage) = self.usage.take() {
            let mut payload = self.base_chunk();
            let mut object = JsonMap::new();
            if let Some(obj) = payload.as_object_mut() {
                object = std::mem::take(obj);
            }
            object.insert("choices".to_string(), json!([]));
            object.insert("usage".to_string(), usage);
            let usage_payload = Value::Object(object);
            chunks.push(chat_sse_chunk(&usage_payload));
        }
        chunks.push(done_marker());
        chunks
    }

    fn ensure_started(&mut self) {
        if self.response_started {
            return;
        }
        // 上游没发 response.created（违规流）：合成首个 role chunk 保证客户端状态机可用
        self.response_started = true;
    }

    /// 统一事件入口。事件名优先 event: 行，缺省回落 data.type。
    fn handle_event(&mut self, event_name: &str, data: &Value) -> Vec<Bytes> {
        let event_type = data
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or(event_name);
        match event_type {
            "response.created" => self.handle_created(data),
            "response.output_item.added" => self.handle_item_added(data),
            "response.output_item.done" | "response.output_item.completed" => {
                self.handle_item_done(data)
            }
            "response.output_text.delta" => {
                let delta = data.get("delta").and_then(Value::as_str).unwrap_or("");
                if delta.is_empty() {
                    Vec::new()
                } else {
                    self.ensure_started();
                    vec![self.choice_chunk(json!({ "content": delta }), None)]
                }
            }
            // 标准 Responses 的 reasoning 摘要增量；BigModel 私有变体同轨处理
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                let delta = data.get("delta").and_then(Value::as_str).unwrap_or("");
                if delta.is_empty() {
                    Vec::new()
                } else {
                    self.ensure_started();
                    vec![self.choice_chunk(json!({ "reasoning_content": delta }), None)]
                }
            }
            "response.function_call_arguments.delta" => self.handle_tool_arguments_delta(data),
            "response.completed" => {
                let response = data.get("response").unwrap_or(data);
                self.usage = Some(super::transform_codex_chat::responses_usage_to_chat_usage(
                    response.get("usage"),
                ));
                let finish = if self.next_tool_index > 0 {
                    "tool_calls"
                } else if response.get("status").and_then(Value::as_str) == Some("incomplete") {
                    "length"
                } else {
                    "stop"
                };
                self.finalize_chunks(finish)
            }
            "response.incomplete" => {
                let response = data.get("response").unwrap_or(data);
                self.usage = Some(super::transform_codex_chat::responses_usage_to_chat_usage(
                    response.get("usage"),
                ));
                self.finalize_chunks("length")
            }
            "response.failed" => {
                let response = data.get("response").unwrap_or(data);
                self.usage = Some(super::transform_codex_chat::responses_usage_to_chat_usage(
                    response.get("usage"),
                ));
                if let Some(error) = response.get("error").filter(|v| !v.is_null()) {
                    log::debug!("[ResponsesToChat] 上游 response.failed: {error}");
                }
                self.finalize_chunks("stop")
            }
            "error" => {
                log::debug!("[ResponsesToChat] 上游 error 事件: {data}");
                self.finalize_chunks("stop")
            }
            // content_part / output_text.done / reasoning summary done 等生命周期
            // 事件对 Chat delta 模型无增量语义，忽略
            _ => Vec::new(),
        }
    }
}

/// Create a stream that converts Responses API SSE events into Chat SSE chunks.
pub fn create_chat_sse_stream_from_responses<E: std::error::Error + Send + 'static>(
    stream: impl Stream<Item = Result<Bytes, E>> + Send + 'static,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send {
    async_stream::stream! {
        let mut buffer = String::new();
        let mut utf8_remainder: Vec<u8> = Vec::new();
        let mut state = ResponsesToChatState::default();

        tokio::pin!(stream);

        while let Some(chunk) = stream.next().await {
            let bytes = match chunk {
                Ok(bytes) => bytes,
                Err(error) => {
                    log::debug!("[ResponsesToChat] 上游流错误: {error}");
                    break;
                }
            };
            crate::proxy::sse::append_utf8_safe(&mut buffer, &mut utf8_remainder, &bytes);

            while let Some(block) = take_sse_block(&mut buffer) {
                if block.trim().is_empty() {
                    continue;
                }

                let mut event_name: Option<String> = None;
                let mut data_parts: Vec<String> = Vec::new();
                for line in block.lines() {
                    if let Some(event) = strip_sse_field(line, "event") {
                        event_name = Some(event.trim().to_string());
                    }
                    if let Some(data) = strip_sse_field(line, "data") {
                        data_parts.push(data.to_string());
                    }
                }

                if data_parts.is_empty() {
                    continue;
                }

                let data = data_parts.join("\n");
                if data.trim() == "[DONE]" {
                    for event in state.finalize_chunks("stop") {
                        yield Ok(event);
                    }
                    continue;
                }

                let data: Value = match serde_json::from_str(&data) {
                    Ok(value) => value,
                    Err(error) => {
                        log::debug!("[ResponsesToChat] 无法解析 SSE 事件体: {error}");
                        continue;
                    }
                };

                let name = event_name.as_deref().unwrap_or("");
                for event in state.handle_event(name, &data) {
                    yield Ok(event);
                }
            }
        }

        // 上游流结束但没收到完成事件（中断/超时）：补 finish + [DONE]，
        // 已缓存的增量已实时下发，客户端至少能正常收尾
        for event in state.finalize_chunks("stop") {
            yield Ok(event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sse_block(event: &str, data: &Value) -> String {
        format!(
            "event: {event}\ndata: {}\n\n",
            serde_json::to_string(data).unwrap()
        )
    }

    fn parse_chunks(bytes: &[Bytes]) -> Vec<Value> {
        let mut values = Vec::new();
        for chunk in bytes {
            let text = String::from_utf8_lossy(chunk);
            for line in text.lines() {
                if let Some(data) = line.strip_prefix("data: ") {
                    if data.trim() == "[DONE]" {
                        continue;
                    }
                    values.push(serde_json::from_str(data).expect("parse chunk json"));
                }
            }
        }
        values
    }

    #[test]
    fn text_delta_stream_produces_chat_chunks() {
        let mut state = ResponsesToChatState::default();
        let mut out = Vec::new();
        out.extend(state.handle_event(
            "response.created",
            &json!({"type":"response.created","response":{"id":"resp_1","model":"glm-5.3","created_at":100}}),
        ));
        out.extend(state.handle_event(
            "response.output_text.delta",
            &json!({"type":"response.output_text.delta","delta":"Hello"}),
        ));
        out.extend(state.handle_event(
            "response.output_text.delta",
            &json!({"type":"response.output_text.delta","delta":" world"}),
        ));
        out.extend(state.handle_event(
            "response.completed",
            &json!({"type":"response.completed","response":{"id":"resp_1","status":"completed","usage":{"input_tokens":3,"output_tokens":4,"total_tokens":7}}}),
        ));

        let chunks = parse_chunks(&out);
        assert_eq!(chunks.len(), 5, "role + 2 deltas + finish + usage");
        assert_eq!(chunks[0]["choices"][0]["delta"]["role"], json!("assistant"));
        assert_eq!(chunks[1]["choices"][0]["delta"]["content"], json!("Hello"));
        assert_eq!(chunks[2]["choices"][0]["delta"]["content"], json!(" world"));
        assert_eq!(chunks[3]["choices"][0]["finish_reason"], json!("stop"));
        // usage chunk（choices 为空、usage 在顶层）
        assert_eq!(chunks[4]["choices"], json!([]));
        assert_eq!(chunks[4]["usage"]["prompt_tokens"], json!(3));
        // 最后的 [DONE]
        let last = String::from_utf8_lossy(out.last().unwrap());
        assert!(last.contains("[DONE]"), "last: {last}");
    }

    #[test]
    fn completed_emits_usage_chunk() {
        let mut state = ResponsesToChatState::default();
        let out = state.handle_event(
            "response.completed",
            &json!({"type":"response.completed","response":{"status":"completed",
                "usage":{"input_tokens":5,"output_tokens":6,"total_tokens":11}}}),
        );
        let chunks = parse_chunks(&out);
        // finish chunk + usage chunk
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[1]["choices"], json!([]));
        assert_eq!(chunks[1]["usage"]["prompt_tokens"], json!(5));
        assert_eq!(chunks[1]["usage"]["completion_tokens"], json!(6));
    }

    #[test]
    fn function_call_lifecycle_maps_tool_calls() {
        let mut state = ResponsesToChatState::default();
        let mut out = Vec::new();
        out.extend(state.handle_event(
            "response.output_item.added",
            &json!({"type":"response.output_item.added","output_index":1,
                "item":{"type":"function_call","id":"fc_1","call_id":"call_abc","name":"get_weather","arguments":""}}),
        ));
        out.extend(state.handle_event(
            "response.function_call_arguments.delta",
            &json!({"type":"response.function_call_arguments.delta","output_index":1,"delta":"{\"ci"}), 
        ));
        out.extend(state.handle_event(
            "response.function_call_arguments.delta",
            &json!({"type":"response.function_call_arguments.delta","output_index":1,"delta":"ty\":\"sz\"}"}),
        ));
        out.extend(state.handle_event(
            "response.output_item.done",
            &json!({"type":"response.output_item.done","output_index":1,
                "item":{"type":"function_call","id":"fc_1","call_id":"call_abc","name":"get_weather","arguments":"{\"city\":\"sz\"}"}}),
        ));
        out.extend(state.handle_event(
            "response.completed",
            &json!({"type":"response.completed","response":{"status":"completed"}}),
        ));

        let chunks = parse_chunks(&out);
        // 首帧 tool_call + 2 个 arguments 增量 + finish
        let first = &chunks[0]["choices"][0]["delta"]["tool_calls"][0];
        assert_eq!(first["id"], json!("call_abc"));
        assert_eq!(first["function"]["name"], json!("get_weather"));
        assert_eq!(chunks[1]["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"], json!("{\"ci"));
        assert_eq!(chunks[2]["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"], json!("ty\":\"sz\"}"));
        assert_eq!(chunks[3]["choices"][0]["finish_reason"], json!("tool_calls"));
    }

    #[test]
    fn multiple_function_calls_get_sequential_indices() {
        let mut state = ResponsesToChatState::default();
        let mut out = Vec::new();
        for (output_index, call_id) in [(0u64, "call_a"), (1u64, "call_b")] {
            out.extend(state.handle_event(
                "response.output_item.added",
                &json!({"type":"response.output_item.added","output_index":output_index,
                    "item":{"type":"function_call","call_id":call_id,"name":"f","arguments":""}}),
            ));
        }
        let chunks = parse_chunks(&out);
        assert_eq!(
            chunks[0]["choices"][0]["delta"]["tool_calls"][0]["index"],
            json!(0)
        );
        assert_eq!(
            chunks[1]["choices"][0]["delta"]["tool_calls"][0]["index"],
            json!(1)
        );
    }

    #[test]
    fn reasoning_delta_supports_standard_and_bigmodel_event_names() {
        let mut state = ResponsesToChatState::default();
        let standard = state.handle_event(
            "response.reasoning_summary_text.delta",
            &json!({"type":"response.reasoning_summary_text.delta","delta":"thinking"}),
        );
        assert_eq!(
            parse_chunks(&standard)[0]["choices"][0]["delta"]["reasoning_content"],
            json!("thinking")
        );

        let mut state = ResponsesToChatState::default();
        let bigmodel = state.handle_event(
            "response.reasoning_text.delta",
            &json!({"type":"response.reasoning_text.delta","delta":"hmm"}),
        );
        assert_eq!(
            parse_chunks(&bigmodel)[0]["choices"][0]["delta"]["reasoning_content"],
            json!("hmm")
        );
    }

    #[test]
    fn incomplete_maps_to_length_finish() {
        let mut state = ResponsesToChatState::default();
        let out = state.handle_event(
            "response.incomplete",
            &json!({"type":"response.incomplete","response":{"status":"incomplete",
                "incomplete_details":{"reason":"max_output_tokens"}}}),
        );
        let chunks = parse_chunks(&out);
        assert_eq!(chunks[0]["choices"][0]["finish_reason"], json!("length"));
    }

    #[test]
    fn failed_and_error_events_close_the_stream() {
        let mut state = ResponsesToChatState::default();
        let out = state.handle_event(
            "response.failed",
            &json!({"type":"response.failed","response":{"error":{"message":"boom"}}}),
        );
        let chunks = parse_chunks(&out);
        assert_eq!(chunks[0]["choices"][0]["finish_reason"], json!("stop"));
        assert!(String::from_utf8_lossy(out.last().unwrap()).contains("[DONE]"));

        // error 事件（无 response 包装）
        let mut state = ResponsesToChatState::default();
        let out = state.handle_event("error", &json!({"type":"error","message":"bad"}));
        assert!(String::from_utf8_lossy(out.last().unwrap()).contains("[DONE]"));
    }

    #[test]
    fn lone_nameless_function_call_fails_closed() {
        let mut state = ResponsesToChatState::default();
        state.handle_event(
            "response.output_item.added",
            &json!({"type":"response.output_item.added","output_index":0,
                "item":{"type":"function_call","call_id":"c1","arguments":""}}),
        );
        let out = state.handle_event(
            "response.output_item.done",
            &json!({"type":"response.output_item.done","output_index":0,
                "item":{"type":"function_call","call_id":"c1","arguments":"{}"}}),
        );
        let finish = state.handle_event(
            "response.completed",
            &json!({"type":"response.completed","response":{"status":"completed"}}),
        );
        let all: Vec<Bytes> = out.into_iter().chain(finish).collect();
        let text = all
            .iter()
            .map(|b| String::from_utf8_lossy(b).to_string())
            .collect::<String>();
        assert!(text.contains("without a name"), "text: {text}");
    }

    #[test]
    fn stream_without_created_event_synthesizes_role_chunk() {
        // 上游直接从 delta 开始（违规流）：仍能产生合法 chunk 序列
        let mut state = ResponsesToChatState::default();
        let out = state.handle_event(
            "response.output_text.delta",
            &json!({"type":"response.output_text.delta","delta":"hi"}),
        );
        let chunks = parse_chunks(&out);
        assert_eq!(chunks[0]["choices"][0]["delta"]["content"], json!("hi"));
        assert_eq!(chunks[0]["object"], json!("chat.completion.chunk"));
    }

    #[tokio::test]
    async fn end_to_end_stream_conversion() {
        let upstream = [
            sse_block(
                "response.created",
                &json!({"type":"response.created","response":{"id":"resp_9","model":"glm-5.3","created_at":42}}),
            ),
            sse_block(
                "response.output_text.delta",
                &json!({"type":"response.output_text.delta","delta":"你好"}),
            ),
            sse_block(
                "response.completed",
                &json!({"type":"response.completed","response":{"status":"completed",
                    "usage":{"input_tokens":1,"output_tokens":2,"total_tokens":3}}}),
            ),
        ]
        .concat();

        let input = futures::stream::iter(vec![Ok::<Bytes, std::io::Error>(Bytes::from(upstream))]);
        let converted: Vec<Bytes> = create_chat_sse_stream_from_responses(input)
            .map(|item| item.expect("converted chunk"))
            .collect()
            .await;

        let text = converted
            .iter()
            .map(|b| String::from_utf8_lossy(b).to_string())
            .collect::<String>();
        assert!(text.contains("\"role\":\"assistant\""));
        assert!(text.contains("你好"));
        assert!(text.contains("\"finish_reason\":\"stop\""));
        assert!(text.contains("\"prompt_tokens\":1"));
        assert!(text.contains("[DONE]"));
    }
}
