// src/mock_random.rs
// Модуль рандомизации плейсхолдеров {{...}} для BunnyProx.
// Автономный, только std, edition 2024.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

// --- PRNG: xorshift64* ---
static RNG_STATE: AtomicU64 = AtomicU64::new(0);

fn init_rng() {
    if RNG_STATE.load(Ordering::Relaxed) == 0 {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        let seed = nanos ^ 0x9E3779B97F4A7C15;
        RNG_STATE.store(seed, Ordering::Relaxed);
    }
}

fn next_u64() -> u64 {
    init_rng();
    let mut x = RNG_STATE.load(Ordering::Relaxed);
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    RNG_STATE.store(x, Ordering::Relaxed);
    x.wrapping_mul(0x2545F4914F6CDD1D)
}

fn next_u32() -> u32 {
    (next_u64() >> 32) as u32
}

fn rand_range(min: i64, max: i64) -> i64 {
    if min > max {
        return rand_range(max, min);
    }
    let range = (max - min + 1) as u64;
    let r = next_u64() % range;
    min + r as i64
}

fn rand_bool() -> bool {
    (next_u32() & 1) != 0
}

fn rand_hex(n: usize) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut s = String::with_capacity(n);
    for _ in 0..n {
        s.push(HEX[(next_u32() % 16) as usize] as char);
    }
    s
}

fn rand_string(n: usize) -> String {
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut s = String::with_capacity(n);
    for _ in 0..n {
        s.push(CHARS[(next_u32() % CHARS.len() as u32) as usize] as char);
    }
    s
}

// --- Встроенные списки ---
const NAMES: &[&str] = &[
    "Иванов Иван Иванович",
    "Петров Петр Петрович",
    "Сидорова Мария Александровна",
    "Смирнов Алексей Владимирович",
    "Кузнецова Елена Сергеевна",
    "Попов Дмитрий Андреевич",
    "Соколова Анна Игоревна",
    "Михайлов Сергей Николаевич",
    "Новикова Ольга Павловна",
    "Волков Артем Олегович",
];

fn rand_name() -> String {
    NAMES[(next_u32() % NAMES.len() as u32) as usize].to_string()
}

fn rand_email() -> String {
    format!("{}@example.com", rand_string(8))
}

fn rand_phone() -> String {
    format!(
        "+7 9{:02} {:03}-{:02}-{:02}",
        next_u32() % 100,
        next_u32() % 1000,
        next_u32() % 100,
        next_u32() % 100
    )
}

fn rand_inn() -> String {
    let first = (next_u32() % 9 + 1) as u8; // 1..9
    let mut s = String::with_capacity(10);
    s.push((b'0' + first) as char);
    for _ in 1..10 {
        s.push((b'0' + (next_u32() % 10) as u8) as char);
    }
    s
}

fn rand_snils() -> String {
    format!(
        "{:03}-{:03}-{:02} {:02}",
        next_u32() % 1000,
        next_u32() % 1000,
        next_u32() % 100,
        next_u32() % 100
    )
}

fn rand_guid() -> String {
    // UUID v4: xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx
    // y: variant bits 10xx (8,9,a,b)
    let mut bytes = [0u8; 16];
    for b in &mut bytes {
        *b = next_u32() as u8;
    }
    bytes[6] = (bytes[6] & 0x0F) | 0x40; // version 4
    bytes[8] = (bytes[8] & 0x3F) | 0x80; // variant 10xx
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5],
        bytes[6], bytes[7],
        bytes[8], bytes[9],
        bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    )
}

fn rand_date(offset: i64) -> String {
    use std::time::{Duration, SystemTime};
    let now = SystemTime::now();
    let target = if offset >= 0 {
        now + Duration::from_secs(offset as u64 * 86400)
    } else {
        now - Duration::from_secs((-offset) as u64 * 86400)
    };
    let secs = target.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    // Простое форматирование YYYY-MM-DD через chrono не доступно, делаем вручную
    // Используем стандартную библиотеку: переводим в tm через gmtime (unsafe) — проще: форматируем через time crate? Нельзя.
    // Реализуем простой расчёт дней от эпохи.
    let days = secs / 86400;
    let mut y = 1970;
    let mut d = days as i64;
    loop {
        let dy = if is_leap(y) { 366 } else { 365 };
        if d < dy { break; }
        d -= dy;
        y += 1;
    }
    let mut m = 1;
    let dim = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut dim2 = dim;
    if is_leap(y) { dim2[1] = 29; }
    while d >= dim2[m - 1] {
        d -= dim2[m - 1];
        m += 1;
    }
    format!("{:04}-{:02}-{:02}", y, m, d + 1)
}

fn is_leap(y: i64) -> bool {
    y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
}

fn rand_datetime(offset: i64) -> String {
    let date = rand_date(offset);
    let h = next_u32() % 24;
    let m = next_u32() % 60;
    let s = next_u32() % 60;
    format!("{}T{:02}:{:02}:{:02}", date, h, m, s)
}

fn rand_float(min: f64, max: f64, dec: u32) -> String {
    if min > max {
        return rand_float(max, min, dec);
    }
    let range = max - min;
    let r = next_u64() as f64 / u64::MAX as f64;
    let val = min + r * range;
    let factor = 10f64.powi(dec as i32);
    let rounded = (val * factor).round() / factor;
    format!("{:.prec$}", rounded, prec = dec as usize)
}

fn rand_pick(options: &[&str]) -> String {
    if options.is_empty() { return String::new(); }
    options[(next_u32() % options.len() as u32) as usize].to_string()
}

// --- Парсинг токена ---
fn parse_token(content: &str) -> Option<String> {
    let content = content.trim();
    if content.is_empty() { return None; }

    // {{int:MIN:MAX}}
    if let Some(rest) = content.strip_prefix("int:") {
        let parts: Vec<&str> = rest.split(':').collect();
        if parts.len() == 2 {
            if let (Ok(min), Ok(max)) = (parts[0].parse::<i64>(), parts[1].parse::<i64>()) {
                return Some(rand_range(min, max).to_string());
            }
        }
    }
    // {{float:MIN:MAX:DEC}}
    else if let Some(rest) = content.strip_prefix("float:") {
        let parts: Vec<&str> = rest.split(':').collect();
        if parts.len() == 3 {
            if let (Ok(min), Ok(max), Ok(dec)) = (parts[0].parse::<f64>(), parts[1].parse::<f64>(), parts[2].parse::<u32>()) {
                if dec <= 9 {
                    return Some(rand_float(min, max, dec));
                }
            }
        }
    }
    // {{bool}}
    else if content.eq_ignore_ascii_case("bool") {
        return Some(rand_bool().to_string());
    }
    // {{guid}}
    else if content.eq_ignore_ascii_case("guid") {
        return Some(rand_guid());
    }
    // {{hex:N}}
    else if let Some(rest) = content.strip_prefix("hex:") {
        if let Ok(n) = rest.parse::<usize>() {
            return Some(rand_hex(n));
        }
    }
    // {{string:N}}
    else if let Some(rest) = content.strip_prefix("string:") {
        if let Ok(n) = rest.parse::<usize>() {
            return Some(rand_string(n));
        }
    }
    // {{email}}
    else if content.eq_ignore_ascii_case("email") {
        return Some(rand_email());
    }
    // {{phone}}
    else if content.eq_ignore_ascii_case("phone") {
        return Some(rand_phone());
    }
    // {{inn}}
    else if content.eq_ignore_ascii_case("inn") {
        return Some(rand_inn());
    }
    // {{snils}}
    else if content.eq_ignore_ascii_case("snils") {
        return Some(rand_snils());
    }
    // {{name}}
    else if content.eq_ignore_ascii_case("name") {
        return Some(rand_name());
    }
    // {{date:OFFSET}}
    else if let Some(rest) = content.strip_prefix("date:") {
        if let Ok(offset) = rest.parse::<i64>() {
            return Some(rand_date(offset));
        }
    }
    // {{datetime:OFFSET}}
    else if let Some(rest) = content.strip_prefix("datetime:") {
        if let Ok(offset) = rest.parse::<i64>() {
            return Some(rand_datetime(offset));
        }
    }
    // {{pick:A|B|C}} — пробелы вокруг значений не значимы, режем
    else if let Some(rest) = content.strip_prefix("pick:") {
        let options: Vec<&str> = rest.split('|').map(|o| o.trim()).collect();
        return Some(rand_pick(&options));
    }

    None // неизвестный токен — оставим как есть
}

/// Заменяет все токены {{...}} в тексте. Неизвестный токен остаётся как есть.
pub fn randomize_text(src: &str) -> String {
    if src.is_empty() { return String::new(); }
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    let bytes = src.as_bytes();
    while i < bytes.len() {
        if i + 3 < bytes.len() && bytes[i] == b'{' && bytes[i+1] == b'{' {
            // ищем закрывающие }}
            let mut j = i + 2;
            while j + 1 < bytes.len() && !(bytes[j] == b'}' && bytes[j+1] == b'}') {
                j += 1;
            }
            if j + 1 < bytes.len() {
                // токен найден
                let token_content = std::str::from_utf8(&bytes[i+2..j]).unwrap_or("");
                if let Some(replacement) = parse_token(token_content) {
                    out.push_str(&replacement);
                } else {
                    // неизвестный — оставляем как есть
                    out.push_str("{{");
                    out.push_str(token_content);
                    out.push_str("}}");
                }
                i = j + 2;
                continue;
            }
        }
        out.push_str(&src[i..i + utf8_len(bytes[i])]);
        i += utf8_len(bytes[i]);
    }
    out
}

/// Длина UTF-8 последовательности по ведущему байту.
fn utf8_len(b: u8) -> usize {
    if b < 0x80 { 1 } else if b >> 5 == 0b110 { 2 } else if b >> 4 == 0b1110 { 3 } else if b >> 3 == 0b11110 { 4 } else { 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t1_int_boundaries() {
        let s = "{{int:5:5}}";
        let r = randomize_text(s);
        assert_eq!(r, "5");
        let s = "{{int:10:5}}"; // swap
        let r = randomize_text(s);
        let v: i64 = r.parse().unwrap();
        assert!((5..=10).contains(&v));
    }

    #[test]
    fn t2_unknown_token_preserved() {
        let s = "{{unknown}}";
        let r = randomize_text(s);
        assert_eq!(r, "{{unknown}}");
    }

    #[test]
    fn t3_guid_format() {
        let s = "{{guid}}";
        let r = randomize_text(s);
        // 8-4-4-4-12 hex
        assert_eq!(r.len(), 36);
        assert_eq!(r.chars().filter(|c| *c == '-').count(), 4);
        // version 4 at pos 14 (0-based)
        assert_eq!(r.chars().nth(14), Some('4'));
        // variant at pos 19: 8,9,a,b
        let v = r.chars().nth(19).unwrap();
        assert!(matches!(v, '8' | '9' | 'a' | 'b'));
    }

    #[test]
    fn t4_string_length() {
        let s = "{{string:12}}";
        let r = randomize_text(s);
        assert_eq!(r.len(), 12);
        assert!(r.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
    }

    #[test]
    fn t5_pick_from_list() {
        let s = "{{pick:A|B|C}}";
        let r = randomize_text(s);
        assert!(matches!(r.as_str(), "A" | "B" | "C"));
        // пробелы вокруг значений режутся: {{pick: A | B }} == {{pick:A|B}}
        let s2 = "{{pick: A | B }}";
        let r2 = randomize_text(s2);
        assert!(matches!(r2.as_str(), "A" | "B"));
    }

    #[test]
    fn t6_float_format() {
        let s = "{{float:1.0:2.0:3}}";
        let r = randomize_text(s);
        // формат с 3 знаками
        assert!(r.contains('.'));
        let parts: Vec<&str> = r.split('.').collect();
        assert_eq!(parts[1].len(), 3);
        let v: f64 = r.parse().unwrap();
        assert!((1.0..=2.0).contains(&v));
    }

    #[test]
    fn t7_date_shift() {
        let s = "{{date:0}}";
        let r = randomize_text(s);
        // YYYY-MM-DD
        assert_eq!(r.len(), 10);
        assert_eq!(r.chars().nth(4), Some('-'));
        assert_eq!(r.chars().nth(7), Some('-'));
        // сегодня + 1 день
        let s2 = "{{date:1}}";
        let r2 = randomize_text(s2);
        assert_ne!(r, r2);
    }

    #[test]
    fn t8_empty_input() {
        let r = randomize_text("");
        assert_eq!(r, "");
    }

    #[test]
    fn t9_multiple_tokens() {
        let s = "id={{guid}}, n={{int:1:3}}, flag={{bool}}";
        let r = randomize_text(s);
        assert!(r.starts_with("id="));
        assert!(r.contains(", n="));
        assert!(r.contains(", flag="));
    }

    #[test]
    fn t10_datetime_format() {
        let s = "{{datetime:0}}";
        let r = randomize_text(s);
        // YYYY-MM-DDTHH:MM:SS
        assert_eq!(r.len(), 19);
        assert_eq!(r.chars().nth(10), Some('T'));
        assert_eq!(r.chars().nth(13), Some(':'));
        assert_eq!(r.chars().nth(16), Some(':'));
    }

    #[test]
    fn t11_utf8_outside_tokens() {
        let s = "{\"ФИО\":\"{{name}}\",\"город\":\"Москва\"}";
        let r = randomize_text(s);
        assert!(r.contains("{\"ФИО\":\""));
        assert!(r.contains("\"город\":\"Москва\"}"));
        assert!(!r.contains("{{name}}"));
    }
}
