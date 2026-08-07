#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
pub const SPINNER_FRAMES: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];

pub const PROGRESS_FULL: char = '█';
pub const PROGRESS_EMPTY: char = '░';

#[derive(Debug, Clone)]
pub enum ActivityType {
    Researching,
    Searching,
    Analysing,
    Planning,
    Building,
    Testing,
    Reviewing,
    Thinking,
    Executing,
}

impl ActivityType {
    pub fn label(&self) -> &'static str {
        match self {
            ActivityType::Researching => "Researching...",
            ActivityType::Searching => "Searching...",
            ActivityType::Analysing => "Analysing...",
            ActivityType::Planning => "Planning...",
            ActivityType::Building => "Building...",
            ActivityType::Testing => "Testing...",
            ActivityType::Reviewing => "Reviewing...",
            ActivityType::Thinking => "Thinking...",
            ActivityType::Executing => "Executing...",
        }
    }
}

pub const FRAME_MS: u64 = 80;

#[derive(Debug, Clone)]
pub struct AnimationState {
    pub frame: usize,
    pub started_at: std::time::Instant,
    pub last_tick: std::time::Instant,
    pub activity: Option<ActivityType>,
}

impl AnimationState {
    pub fn new() -> Self {
        AnimationState {
            frame: 0,
            started_at: std::time::Instant::now(),
            last_tick: std::time::Instant::now(),
            activity: None,
        }
    }

    pub fn start_activity(&mut self, activity: ActivityType) {
        self.activity = Some(activity);
        self.frame = 0;
        self.started_at = std::time::Instant::now();
        self.last_tick = std::time::Instant::now();
    }

    pub fn stop_activity(&mut self) {
        self.activity = None;
    }

    pub fn is_active(&self) -> bool {
        self.activity.is_some()
    }

    /// Advances the spinner only if `FRAME_MS` elapsed since the last tick.
    /// Returns true when the frame actually advanced (caller may redraw).
    pub fn tick_if_due(&mut self) -> bool {
        if !self.is_active() {
            return false;
        }
        if self.last_tick.elapsed().as_millis() as u64 >= FRAME_MS {
            self.frame = (self.frame + 1) % SPINNER_FRAMES.len();
            self.last_tick = std::time::Instant::now();
            true
        } else {
            false
        }
    }

    pub fn tick(&mut self) {
        self.frame = (self.frame + 1) % SPINNER_FRAMES.len();
        self.last_tick = std::time::Instant::now();
    }

    pub fn spinner_char(&self) -> &'static str {
        SPINNER_FRAMES[self.frame]
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis() as u64
    }

    pub fn current_label(&self) -> &'static str {
        match &self.activity {
            Some(activity) => activity.label(),
            None => "",
        }
    }
}

impl Default for AnimationState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn progress_bar(progress: f32, width: usize) -> String {
    let progress = progress.clamp(0.0, 1.0);
    let filled = (progress * width as f32).round() as usize;
    let empty = width.saturating_sub(filled);

    let mut bar = String::new();
    for _ in 0..filled {
        bar.push(PROGRESS_FULL);
    }
    for _ in 0..empty {
        bar.push(PROGRESS_EMPTY);
    }
    bar
}

pub fn percentage(progress: f32) -> String {
    let progress = progress.clamp(0.0, 1.0);
    format!("{:>3}%", (progress * 100.0).round() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spinner_cycles() {
        let mut anim = AnimationState::new();
        anim.start_activity(ActivityType::Thinking);
        let first = anim.spinner_char();
        anim.tick();
        let second = anim.spinner_char();
        assert_ne!(first, second);
    }

    #[test]
    fn test_progress_bar() {
        let bar = progress_bar(0.5, 10);
        assert_eq!(bar.chars().count(), 10);
        assert_eq!(bar.chars().filter(|&c| c == '█').count(), 5);
        assert_eq!(bar.chars().filter(|&c| c == '░').count(), 5);
    }

    #[test]
    fn test_progress_clamped() {
        let bar = progress_bar(2.0, 10);
        assert_eq!(bar.chars().filter(|&c| c == '█').count(), 10);

        let bar = progress_bar(-1.0, 10);
        assert_eq!(bar.chars().filter(|&c| c == '█').count(), 0);
    }

    #[test]
    fn test_percentage() {
        assert_eq!(percentage(0.5), " 50%");
        assert_eq!(percentage(1.0), "100%");
        assert_eq!(percentage(0.0), "  0%");
    }

    #[test]
    fn test_activity_label() {
        assert_eq!(ActivityType::Searching.label(), "Searching...");
        assert_eq!(ActivityType::Building.label(), "Building...");
        assert_eq!(ActivityType::Testing.label(), "Testing...");
    }
}
