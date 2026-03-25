use figlet_rs::FIGlet;
use std::env;

pub const BITLOOPS_PURPLE_HEX: &str = "#7404e4";

pub fn should_use_color_output() -> bool {
    env::var_os("NO_COLOR").is_none() && env::var("ACCESSIBLE").is_err()
}

pub fn color_hex(text: &str, hex: &str) -> String {
    let hex = hex.trim_start_matches('#');
    let r = u8::from_str_radix(hex.get(0..2).unwrap_or("00"), 16).unwrap_or(0);
    let g = u8::from_str_radix(hex.get(2..4).unwrap_or("00"), 16).unwrap_or(0);
    let b = u8::from_str_radix(hex.get(4..6).unwrap_or("00"), 16).unwrap_or(0);
    format!("\x1b[38;2;{r};{g};{b}m{text}\x1b[0m")
}

pub fn color_hex_if_enabled(text: &str, hex: &str) -> String {
    if should_use_color_output() {
        color_hex(text, hex)
    } else {
        text.to_string()
    }
}

pub fn squared_capital(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' => char::from_u32(0x1F170 + (c as u32 - 'A' as u32)).unwrap_or(c),
            'a'..='z' => char::from_u32(0x1F170 + (c as u32 - 'a' as u32)).unwrap_or(c),
            _ => c,
        })
        .collect()
}

pub fn bitloops_wordmark() -> String {
    if let Ok(font) = FIGlet::standard()
        && let Some(figure) = font.convert("bitloops")
    {
        return figure.to_string().trim_end().to_string();
    }

    squared_capital("Bitloops")
}
