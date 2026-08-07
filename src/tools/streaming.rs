//! Tool Streaming Support
//!
//! Defines the `AsyncTool` trait and `StreamResult` type for tools that produce
//! output incrementally rather than as a single string.

use anyhow::Result;
use futures::stream::{self, Stream, StreamExt};
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::mpsc;

use super::context::ToolContext;

/// A chunk of streaming output from a tool.
#[derive(Debug, Clone)]
pub struct StreamChunk {
    /// The text chunk.
    pub text: String,
    /// Whether this is the final chunk.
    pub is_final: bool,
    /// Optional metadata (e.g., "thinking", "tool_output").
    pub metadata: Option<String>,
}

impl StreamChunk {
    pub fn new(text: &str, is_final: bool) -> Self {
        StreamChunk {
            text: text.to_string(),
            is_final,
            metadata: None,
        }
    }

    pub fn final_chunk(text: &str) -> Self {
        StreamChunk {
            text: text.to_string(),
            is_final: true,
            metadata: None,
        }
    }
}

/// Result of a streaming tool execution.
pub struct StreamResult {
    /// The stream of output chunks.
    pub chunks: Pin<Box<dyn Stream<Item = StreamChunk> + Send>>,
    /// Whether the tool supports streaming.
    pub supports_streaming: bool,
    /// Tool name.
    pub tool_name: String,
}

impl std::fmt::Debug for StreamResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamResult")
            .field("supports_streaming", &self.supports_streaming)
            .field("tool_name", &self.tool_name)
            .finish()
    }
}

impl StreamResult {
    pub fn new(chunks: impl Stream<Item = StreamChunk> + Send + 'static, tool_name: &str) -> Self {
        StreamResult {
            chunks: Box::pin(chunks),
            supports_streaming: true,
            tool_name: tool_name.to_string(),
        }
    }

    /// Collect all chunks into a single string.
    pub async fn collect(self) -> Result<String> {
        let mut output = String::new();
        let mut stream = self.chunks;
        while let Some(chunk) = stream.next().await {
            output.push_str(&chunk.text);
            if chunk.is_final {
                break;
            }
        }
        Ok(output)
    }

    /// Convert to a future that resolves to a single string.
    pub async fn into_output(self) -> Result<String> {
        self.collect().await
    }
}

/// Trait for tools that support streaming output.
///
/// Implement this trait in addition to `Tool` when the tool produces output
/// incrementally (e.g., long-running commands, large file reads).
pub trait AsyncTool: Send + Sync {
    /// Tool name (should match the sync name).
    fn name(&self) -> &str;

    /// Stream the tool output.
    fn execute_stream(
        &self,
        args: &str,
        context: &ToolContext,
    ) -> Pin<Box<dyn Future<Output = Result<StreamResult>> + Send>>;
}

use std::future::Future;

/// Convert a sync tool execution into a streaming result (single-chunk stream).
pub fn sync_to_stream(
    tool: &dyn super::Tool,
    args: &str,
    _context: &ToolContext,
) -> Result<StreamResult> {
    let output = tool.execute(args)?;
    let chunks = stream::once(async move { StreamChunk::final_chunk(&output) });
    Ok(StreamResult::new(chunks, tool.name()))
}

/// Create a channel-based stream from a producer closure.
pub fn channel_stream(
    tool_name: &str,
    producer: impl FnOnce(mpsc::Sender<StreamChunk>) -> Result<()> + Send + 'static,
) -> StreamResult {
    let (tx, rx) = mpsc::channel::<StreamChunk>(32);

    std::thread::spawn(move || {
        let _ = producer(tx);
    });

    let stream = stream::unfold(rx, |mut rx| async move {
        match rx.recv().await {
            Some(chunk) => Some((chunk, rx)),
            None => None,
        }
    });

    StreamResult::new(stream, tool_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::runtime::Runtime;

    #[test]
    fn test_stream_chunk() {
        let chunk = StreamChunk::new("hello", false);
        assert_eq!(chunk.text, "hello");
        assert!(!chunk.is_final);

        let final_chunk = StreamChunk::final_chunk("done");
        assert!(final_chunk.is_final);
    }

    #[test]
    fn test_stream_result_collect() {
        let rt = Runtime::new().unwrap();
        rt.block_on(async {
            let chunks = vec![
                StreamChunk::new("part1", false),
                StreamChunk::new("part2", false),
                StreamChunk::final_chunk("part3"),
            ];
            let stream = stream::iter(chunks);
            let result = StreamResult::new(Box::pin(stream), "test");
            let collected = result.collect().await.unwrap();
            assert_eq!(collected, "part1part2part3");
        });
    }

    #[test]
    fn test_sync_to_stream() {
        let rt = Runtime::new().unwrap();
        rt.block_on(async {
            struct DummyTool;
            impl super::super::Tool for DummyTool {
                fn name(&self) -> &str {
                    "dummy"
                }
                fn description(&self) -> &str {
                    "dummy"
                }
                fn execute(&self, _args: &str) -> Result<String> {
                    Ok("sync output".to_string())
                }
            }

            let tool = DummyTool;
            let ctx = ToolContext::new("dummy", "args");
            let stream_result = sync_to_stream(&tool, "args", &ctx).unwrap();
            let collected = stream_result.collect().await.unwrap();
            assert_eq!(collected, "sync output");
        });
    }

    #[test]
    fn test_channel_stream() {
        let rt = Runtime::new().unwrap();
        rt.block_on(async {
            let result = channel_stream("test", |tx| {
                let _ = tx.blocking_send(StreamChunk::new("chunk1", false));
                let _ = tx.blocking_send(StreamChunk::final_chunk("chunk2"));
                Ok(())
            });
            let collected = result.collect().await.unwrap();
            assert_eq!(collected, "chunk1chunk2");
        });
    }
}
