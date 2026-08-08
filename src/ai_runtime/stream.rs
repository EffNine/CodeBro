use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fmt;

use super::response::{ModelResponse, ResponseDelta};
use super::types::{AIRRuntimeError, AIRRuntimeResult};

/// A single segment of a streaming response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamSegment {
    pub delta: ResponseDelta,
    pub index: usize,
    pub finish_reason: Option<String>,
}

impl StreamSegment {
    pub fn new(delta: ResponseDelta, index: usize) -> Self {
        StreamSegment {
            delta,
            index,
            finish_reason: None,
        }
    }

    pub fn with_finish_reason(mut self, reason: impl Into<String>) -> Self {
        self.finish_reason = Some(reason.into());
        self
    }

    pub fn is_finished(&self) -> bool {
        self.finish_reason.is_some()
    }

    pub fn content_fragment(&self) -> Option<&str> {
        self.delta.content.as_deref()
    }
}

/// Events that can occur during streaming.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StreamEvent {
    /// A new segment arrived
    Segment(StreamSegment),
    /// Stream completed successfully
    Complete {
        total_tokens: usize,
        total_duration_ms: u64,
    },
    /// Stream encountered an error
    Error { error: String },
    /// Stream was cancelled
    Cancelled,
}

impl StreamEvent {
    pub fn is_segment(&self) -> bool {
        matches!(self, StreamEvent::Segment(_))
    }

    pub fn is_complete(&self) -> bool {
        matches!(self, StreamEvent::Complete { .. })
    }

    pub fn is_error(&self) -> bool {
        matches!(self, StreamEvent::Error { .. })
    }

    pub fn as_segment(&self) -> Option<&StreamSegment> {
        match self {
            StreamEvent::Segment(s) => Some(s),
            _ => None,
        }
    }
}

impl fmt::Display for StreamEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StreamEvent::Segment(segment) => {
                if let Some(content) = segment.content_fragment() {
                    write!(f, "Segment({}, content=\"{}\")", segment.index, content)
                } else {
                    write!(f, "Segment({})", segment.index)
                }
            }
            StreamEvent::Complete {
                total_tokens,
                total_duration_ms,
            } => {
                write!(
                    f,
                    "Complete(tokens={}, duration={}ms)",
                    total_tokens, total_duration_ms
                )
            }
            StreamEvent::Error { error } => {
                write!(f, "Error({})", error)
            }
            StreamEvent::Cancelled => {
                write!(f, "Cancelled")
            }
        }
    }
}

/// Output from a streaming pipeline.
#[derive(Debug)]
pub struct StreamingOutput {
    pub segments: Vec<StreamSegment>,
    pub total_tokens: usize,
    pub duration_ms: u64,
    pub finished: bool,
}

impl StreamingOutput {
    pub fn new() -> Self {
        StreamingOutput {
            segments: Vec::new(),
            total_tokens: 0,
            duration_ms: 0,
            finished: false,
        }
    }

    pub fn append(&mut self, segment: StreamSegment) {
        self.segments.push(segment);
        self.total_tokens += 1;
    }

    pub fn finish(mut self) -> Self {
        self.finished = true;
        self
    }

    pub fn collect_content(&self) -> String {
        self.segments
            .iter()
            .filter_map(|s| s.content_fragment())
            .collect()
    }
}

impl Default for StreamingOutput {
    fn default() -> Self {
        Self::new()
    }
}

/// A pipeline that processes streaming events into a structured output.
#[derive(Debug)]
pub struct StreamPipeline {
    buffer: VecDeque<StreamEvent>,
}

impl StreamPipeline {
    pub fn new() -> Self {
        StreamPipeline {
            buffer: VecDeque::new(),
        }
    }

    pub fn push(&mut self, event: StreamEvent) {
        self.buffer.push_back(event);
    }

    pub fn pop(&mut self) -> Option<StreamEvent> {
        self.buffer.pop_front()
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    pub fn drain(&mut self) -> Vec<StreamEvent> {
        self.buffer.drain(..).collect()
    }

    /// Process all events and produce a StreamingOutput.
    pub fn process(&mut self) -> AIRRuntimeResult<StreamingOutput> {
        let mut output = StreamingOutput::new();
        let mut tokens = 0usize;
        let mut start_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        while let Some(event) = self.pop() {
            match event {
                StreamEvent::Segment(segment) => {
                    output.append(segment);
                }
                StreamEvent::Complete {
                    total_tokens,
                    total_duration_ms,
                } => {
                    output.total_tokens = total_tokens;
                    output.duration_ms = total_duration_ms;
                    output.finished = true;
                }
                StreamEvent::Error { error } => {
                    return Err(AIRRuntimeError::StreamingError(error));
                }
                StreamEvent::Cancelled => {
                    output.finished = true;
                    return Ok(output);
                }
            }
        }

        if !output.finished {
            let end_time = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            output.duration_ms = end_time.saturating_sub(start_time);
            output.finished = true;
        }

        Ok(output)
    }

    pub fn peek(&self) -> Option<&StreamEvent> {
        self.buffer.front()
    }
}

impl Default for StreamPipeline {
    fn default() -> Self {
        Self::new()
    }
}
