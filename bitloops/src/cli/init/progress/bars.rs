use crate::utils::branding::{BITLOOPS_PURPLE_HEX, color_hex_if_enabled};

pub(crate) const SUCCESS_GREEN_HEX: &str = "#22c55e";

pub(crate) fn determinate_progress_bar_segments(
    width: usize,
    ratio: f64,
    in_memory_ratio: f64,
) -> (usize, usize, usize) {
    let ratio = ratio.clamp(0.0, 1.0);
    let in_memory_ratio = in_memory_ratio.clamp(0.0, ratio);
    let filled = ((width as f64) * ratio).round() as usize;
    let filled = filled.min(width);
    let persisted_ratio = (ratio - in_memory_ratio).clamp(0.0, 1.0);
    let persisted = (((width as f64) * persisted_ratio).round() as usize).min(filled);
    let in_memory = filled.saturating_sub(persisted);
    (persisted, in_memory, width.saturating_sub(filled))
}

pub(crate) fn render_determinate_progress_bar(
    width: usize,
    ratio: f64,
    in_memory_ratio: f64,
) -> String {
    let (persisted, in_memory, empty) =
        determinate_progress_bar_segments(width, ratio, in_memory_ratio);
    let persisted_fill = color_hex_if_enabled(&"█".repeat(persisted), BITLOOPS_PURPLE_HEX);
    let in_memory_fill = color_hex_if_enabled(&"█".repeat(in_memory), SUCCESS_GREEN_HEX);
    let empty = "░".repeat(empty);
    format!("{persisted_fill}{in_memory_fill}{empty}")
}

pub(crate) fn render_indeterminate_progress_bar(width: usize, spinner_index: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let position = spinner_index % width;
    let prefix = "░".repeat(position);
    let pulse = color_hex_if_enabled("█", BITLOOPS_PURPLE_HEX);
    let suffix = "░".repeat(width.saturating_sub(position + 1));
    format!("{prefix}{pulse}{suffix}")
}

#[cfg(test)]
mod tests {
    use super::determinate_progress_bar_segments;

    #[test]
    fn determinate_progress_bar_segments_place_in_memory_fill_after_persisted_fill() {
        assert_eq!(determinate_progress_bar_segments(10, 0.6, 0.2), (4, 2, 4));
    }
}
