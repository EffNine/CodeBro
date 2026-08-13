//! Centralized visual theme for the CodeBro TUI.
//!
//! Every color and phase/status accent used by the renderer lives here so the
//! UI stays consistent and a future theme can be added without touching the
//! rendering code. The palette is a restrained dark terminal palette; text
//! meaning is never carried by color alone (glyphs stay alongside).

use ratatui::style::{Color, Modifier, Style};

/// The single theme in use. Semantic accessors keep rendering code free of
/// hardcoded colors.
#[derive(Debug, Clone)]
pub struct Theme {
    pub bg: Color,
    pub surface: Color,
    pub border: Color,
    pub primary: Color,
    pub secondary: Color,
    pub muted: Color,
    pub purple: Color,
    pub blue: Color,
    pub green: Color,
    pub yellow: Color,
    pub red: Color,
    pub orange: Color,
    pub cyan: Color,
}

pub const THEME: Theme = Theme {
    bg: Color::Rgb(0x0d, 0x11, 0x17),
    surface: Color::Rgb(0x16, 0x1b, 0x22),
    border: Color::Rgb(0x21, 0x26, 0x2d),
    primary: Color::Rgb(0xc9, 0xd1, 0xd9),
    secondary: Color::Rgb(0x8b, 0x94, 0x9e),
    muted: Color::Rgb(0x6e, 0x76, 0x81),
    purple: Color::Rgb(0xa7, 0x8b, 0xfa),
    blue: Color::Rgb(0x58, 0xa6, 0xff),
    green: Color::Rgb(0x3f, 0xb9, 0x50),
    yellow: Color::Rgb(0xd2, 0x99, 0x22),
    red: Color::Rgb(0xf8, 0x51, 0x49),
    orange: Color::Rgb(0xff, 0xa6, 0x57),
    cyan: Color::Rgb(0x79, 0xc0, 0xff),
};

/// The five autonomous specialist phases plus the main/verification phases.
/// Phase identity drives both the accent color and the emoji vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Phase {
    Research,
    Testing,
    Planning,
    Coding,
    Review,
    Verification,
    Main,
}

impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Phase::Research => "Research",
            Phase::Testing => "Testing",
            Phase::Planning => "Planning",
            Phase::Coding => "Coding",
            Phase::Review => "Review",
            Phase::Verification => "Verification",
            Phase::Main => "Main",
        }
    }

    /// The semantic emoji for the phase. Never the only signal: labels and
    /// glyphs are rendered alongside.
    pub fn emoji(self) -> &'static str {
        match self {
            Phase::Research => "📚",
            Phase::Testing => "🧪",
            Phase::Planning => "🗺",
            Phase::Coding => "💻",
            Phase::Review => "🔍",
            Phase::Verification => "🧪",
            Phase::Main => "🧠",
        }
    }
}

/// Status glyphs shared by phases and actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusGlyph {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
    Warning,
    Ready,
}

impl StatusGlyph {
    pub fn glyph(self) -> &'static str {
        match self {
            StatusGlyph::Pending => "○",
            StatusGlyph::Running => "●",
            StatusGlyph::Completed => "✓",
            StatusGlyph::Failed => "✗",
            StatusGlyph::Cancelled => "⏸",
            StatusGlyph::Warning => "⚠",
            StatusGlyph::Ready => "○",
        }
    }
}

impl Theme {
    pub fn phase_color(&self, phase: Phase) -> Color {
        match phase {
            Phase::Research => self.cyan,
            Phase::Testing => self.green,
            Phase::Planning => self.yellow,
            Phase::Coding => self.purple,
            Phase::Review => self.orange,
            Phase::Verification => self.green,
            Phase::Main => self.purple,
        }
    }

    pub fn cyan(&self) -> Style {
        Style::default().fg(self.cyan)
    }

    pub fn status_color(&self, glyph: StatusGlyph) -> Color {
        match glyph {
            StatusGlyph::Completed | StatusGlyph::Ready => self.green,
            StatusGlyph::Warning => self.yellow,
            StatusGlyph::Failed => self.red,
            StatusGlyph::Pending => self.muted,
            StatusGlyph::Running => self.purple,
            StatusGlyph::Cancelled => self.yellow,
        }
    }

    pub fn block_style(&self) -> Style {
        Style::default().fg(self.border)
    }

    pub fn border_style(&self) -> Style {
        Style::default().fg(self.border)
    }

    pub fn title_style(&self) -> Style {
        Style::default()
            .fg(self.secondary)
            .add_modifier(Modifier::BOLD)
    }

    pub fn text(&self) -> Style {
        Style::default().fg(self.primary)
    }

    pub fn dim(&self) -> Style {
        Style::default().fg(self.muted)
    }

    pub fn secondary(&self) -> Style {
        Style::default().fg(self.secondary)
    }

    pub fn bold(&self) -> Style {
        Style::default()
            .fg(self.primary)
            .add_modifier(Modifier::BOLD)
    }

    pub fn yellow(&self) -> Style {
        Style::default().fg(self.yellow)
    }

    pub fn green(&self) -> Style {
        Style::default().fg(self.green)
    }

    pub fn blue(&self) -> Style {
        Style::default().fg(self.blue)
    }

    pub fn red(&self) -> Style {
        Style::default().fg(self.red)
    }

    pub fn purple(&self) -> Style {
        Style::default().fg(self.purple)
    }

    pub fn orange(&self) -> Style {
        Style::default().fg(self.orange)
    }
}
