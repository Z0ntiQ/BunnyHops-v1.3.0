//! Чистая логика интерфейса — перенос 1:1 хелперов из App.jsx.
//! Тестируется нативно (cargo test), без wasm.

use regex::Regex;

/* ---------- URL ↔ query params (queryToParams / paramsToQuery) ---------- */

#[derive(Clone, Debug, PartialEq)]
pub struct SplitUrl {
    pub base: String,
    pub query: String,
}

pub fn split_url(url: &str) -> SplitUrl {
    match url.find('?') {
        None => SplitUrl {
            base: url.to_string(),
            query: String::new(),
        },
        Some(i) => SplitUrl {
            base: url[..i].to_string(),
            query: url[i + 1..].to_string(),
        },
    }
}

fn safe_decode(s: &str) -> String {
    // decodeURIComponent: percent-decode UTF-8; при ошибке — как есть.
    urlpercent_decode(s).unwrap_or_else(|| s.to_string())
}

fn urlpercent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return None;
            }
            let hi = (bytes[i + 1] as char).to_digit(16)?;
            let lo = (bytes[i + 2] as char).to_digit(16)?;
            out.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct KvRow {
    pub key: String,
    pub value: String,
    pub on: bool,
}

pub fn query_to_params(query: &str) -> Vec<KvRow> {
    if query.is_empty() {
        return vec![];
    }
    query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|part| match part.find('=') {
            None => KvRow {
                key: safe_decode(part),
                value: String::new(),
                on: true,
            },
            Some(j) => KvRow {
                key: safe_decode(&part[..j]),
                value: safe_decode(&part[j + 1..]),
                on: true,
            },
        })
        .collect()
}

/// encodeURIComponent
pub fn js_encode_uri_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.as_bytes() {
        let c = *b as char;
        if c.is_ascii_alphanumeric()
            || matches!(c, '-' | '_' | '.' | '!' | '~' | '*' | '\'' | '(' | ')')
        {
            out.push(c);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn params_to_query(params: &[KvRow]) -> String {
    params
        .iter()
        .filter(|p| p.on && !p.key.trim().is_empty())
        .map(|p| {
            format!(
                "{}={}",
                js_encode_uri_component(&p.key),
                js_encode_uri_component(&p.value)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

/* ---------- helpers (prettyJson / formatSize) ---------- */

pub fn pretty_json(text: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    serde_json::to_string_pretty(&v).ok()
}

pub fn format_size(bytes: Option<u64>) -> String {
    let Some(b) = bytes else {
        return "—".to_string();
    };
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    let f = b as f64;
    if f < KB {
        format!("{b} B")
    } else if f < MB {
        format!("{:.2} KB", f / KB)
    } else {
        format!("{:.2} MB", f / MB)
    }
}

/* ---------- подсветка ошибок в JSON (ERROR_LINE_RE) ---------- */

pub fn error_ranges(line: &str) -> Vec<(usize, usize)> {
    // Дословная копия регулярки из App.jsx.
    let re = Regex::new(r#"(?i)"(?:message|msg|description|detail|details|reason|hint)"\s*:\s*"[^"]*(?:error|not found|invalid|denied|unauthorized|forbidden|expired|insufficient|too many|failed|fail|unsupported|missing|not allowed|does not)[^"]*"|"(?:error|errors|exception|fault|failure)"\s*:\s*"[^"]*""#).unwrap();
    re.find_iter(line)
        .filter(|m| m.as_str().len() > 2)
        .map(|m| (m.start(), m.end()))
        .collect()
}

pub fn search_hits(line: &str, query: &str) -> Vec<(usize, usize)> {
    if query.is_empty() {
        return vec![];
    }
    let low = line.to_lowercase();
    let lq = query.to_lowercase();
    let mut hits = Vec::new();
    let mut i = 0;
    while let Some(p) = low[i..].find(&lq) {
        let s = i + p;
        hits.push((s, s + lq.len()));
        i = s + lq.len();
    }
    hits
}

#[derive(Clone, Debug, PartialEq)]
pub struct Seg {
    pub text: String,
    pub mi: Option<usize>,
    pub err: bool,
}

/// Сегментация без скана JSON-ошибок: для не-JSON тел (HTML и т.п.),
/// где error_ranges только жжёт время на огромных однострочных ответах.
pub fn plain_segments(line: &str, query: &str, next_mi: &mut dyn FnMut() -> usize) -> Vec<Seg> {
    let mut segs: Vec<Seg> = Vec::new();
    search_into(&mut segs, line, query, next_mi, 0, line.len());
    segs
}

/// Построчная сегментация: обычный текст / «мимо» поиска / ошибка.
/// next_mi — счётчик совпадений (замыкается на счётчик вызывающего).
fn push_plain(segs: &mut Vec<Seg>, line: &str, s: usize, e: usize) {
    if e > s {
        segs.push(Seg {
            text: line[s..e].to_string(),
            mi: None,
            err: false,
        });
    }
}

fn search_into(
    segs: &mut Vec<Seg>,
    line: &str,
    query: &str,
    next_mi: &mut dyn FnMut() -> usize,
    s: usize,
    e: usize,
) {
    if e <= s {
        return;
    }
    if query.is_empty() {
        push_plain(segs, line, s, e);
        return;
    }
    let mut pos = s;
    for (hs, he) in search_hits(&line[..e], query) {
        let h_end = he.min(e);
        if h_end < hs + query.len() {
            continue; // сильно обрезанный — пропустим
        }
        if hs > pos {
            push_plain(segs, line, pos, hs);
        }
        segs.push(Seg {
            text: line[hs..h_end].to_string(),
            mi: Some(next_mi()),
            err: false,
        });
        pos = h_end;
    }
    push_plain(segs, line, pos, e);
}

/// Построчная сегментация: обычный текст / «мимо» поиска / ошибка.
pub fn build_segments(line: &str, query: &str, next_mi: &mut dyn FnMut() -> usize) -> Vec<Seg> {
    let mut segs: Vec<Seg> = Vec::new();
    let mut cursor = 0usize;
    for (es, ee) in error_ranges(line) {
        if ee <= cursor {
            continue;
        }
        let start = es.max(cursor);
        search_into(&mut segs, line, query, next_mi, cursor, start);
        if ee > start {
            segs.push(Seg {
                text: line[start..ee].to_string(),
                mi: None,
                err: true,
            });
        }
        cursor = ee.min(line.len());
        if cursor >= line.len() {
            break;
        }
    }
    search_into(&mut segs, line, query, next_mi, cursor, line.len());
    if segs.is_empty() {
        segs.push(Seg {
            text: line.to_string(),
            mi: None,
            err: false,
        });
    }
    segs
}

/* ---------- extractByPath: $.a.b.0.c ---------- */

pub fn extract_by_path(obj: &serde_json::Value, path: &str) -> Result<String, String> {
    // replace(/^\$\??/, '') затем split(/[.[\]"']+/)
    let rest = path
        .strip_prefix("$?")
        .or_else(|| path.strip_prefix('$'))
        .unwrap_or(path);
    let parts: Vec<String> = rest
        .split(['.', '[', ']', '"', '\''])
        .filter(|p| !p.is_empty())
        .map(|p| p.to_string())
        .collect();
    let mut cur = obj;
    for p in &parts {
        let err = || format!("путь «{path}» не найден");
        cur = match cur {
            serde_json::Value::Object(m) => m.get(p.as_str()).ok_or_else(err)?,
            serde_json::Value::Array(a) => {
                let idx: usize = p.parse().map_err(|_| err())?;
                a.get(idx).ok_or_else(err)?
            }
            _ => return Err(err()),
        };
    }
    if cur.is_null() {
        return Err(format!("путь «{path}» не найден"));
    }
    Ok(match cur {
        serde_json::Value::Object(_) | serde_json::Value::Array(_) => cur.to_string(),
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    })
}

/* ---------- шаблонизатор {{name}} ---------- */

pub const VARS_PATTERN: &str = r"\{\{\s*([^}\s{}]+)\s*\}\}";

pub fn tpl_global(v: &str, vars: &std::collections::HashMap<String, String>) -> String {
    let re = Regex::new(VARS_PATTERN).unwrap();
    re.replace_all(v, |caps: &regex::Captures| {
        let name = &caps[1];
        vars.get(name)
            .cloned()
            .unwrap_or_else(|| caps[0].to_string())
    })
    .to_string()
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreviewPart {
    pub text: String,
    pub ok: bool,
}

pub fn url_preview(
    url: &str,
    vars: &std::collections::HashMap<String, String>,
) -> Option<Vec<PreviewPart>> {
    let resolved = tpl_global(url, vars);
    if url == resolved {
        return None;
    }
    let re = Regex::new(VARS_PATTERN).unwrap();
    let mut parts = Vec::new();
    let mut pos = 0;
    for m in re.find_iter(url) {
        if m.start() > pos {
            parts.push(PreviewPart {
                text: url[pos..m.start()].to_string(),
                ok: true,
            });
        }
        let name = re.captures(m.as_str()).unwrap()[1].to_string();
        match vars.get(&name) {
            Some(val) => parts.push(PreviewPart {
                text: val.clone(),
                ok: true,
            }),
            None => parts.push(PreviewPart {
                text: m.as_str().to_string(),
                ok: false,
            }),
        }
        pos = m.end();
    }
    if pos < url.len() {
        parts.push(PreviewPart {
            text: url[pos..].to_string(),
            ok: true,
        });
    }
    Some(parts)
}

/* ---------- масштаб (SCALES) ---------- */

pub const SCALES: [f64; 13] = [
    1.6, 1.5, 1.4, 1.3, 1.2, 1.1, 1.0, 0.9, 0.8, 0.7, 0.6, 0.5, 0.4,
];
pub const DEFAULT_SCALE_IDX: usize = 6;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn params_roundtrip() {
        let s = split_url("https://x.dev/a?b=1&c=%D1%82&noval");
        assert_eq!(s.base, "https://x.dev/a");
        let p = query_to_params(&s.query);
        assert_eq!(p.len(), 3);
        assert_eq!(
            p[1],
            KvRow {
                key: "c".into(),
                value: "т".into(),
                on: true
            }
        );
        assert_eq!(
            p[2],
            KvRow {
                key: "noval".into(),
                value: String::new(),
                on: true
            }
        );
        let q = params_to_query(&p);
        assert_eq!(q, "b=1&c=%D1%82&noval=");
    }

    #[test]
    fn pretty_and_size() {
        assert_eq!(
            pretty_json("{\"a\":1}"),
            Some("{\n  \"a\": 1\n}".to_string())
        );
        assert_eq!(pretty_json("nope"), None);
        assert_eq!(format_size(Some(512)), "512 B");
        assert_eq!(format_size(Some(1536)), "1.50 KB");
        assert_eq!(format_size(None), "—");
    }

    fn plain_segments_no_error_scan() {
        let line = r#"{"error":"boom"}"#;
        let mut mi = 0;
        let segs = plain_segments(line, "boom", &mut || {
            mi += 1;
            mi - 1
        });
        assert!(segs.iter().all(|s| !s.err));
        assert!(segs.iter().any(|s| s.mi == Some(0) && s.text == "boom"));
    }

    #[test]
    #[test]
    fn error_ranges_match_json_error_lines() {
        let line = r#"  "message": "record not found", "ok": true"#;
        let r = error_ranges(line);
        assert_eq!(r.len(), 1);
        assert_eq!(&line[r[0].0..r[0].1], r#""message": "record not found""#);
        let r2 = error_ranges(r#""error": "boom""#);
        assert_eq!(r2.len(), 1);
    }

    #[test]
    fn segments_mark_matches_and_errors() {
        let line = r#""error": "token expired", "note": "token here""#;
        let mut mi = 0usize;
        let segs = build_segments(line, "token", &mut || {
            let v = mi;
            mi += 1;
            v
        });
        assert!(segs.iter().any(|s| s.err));
        let marks: Vec<_> = segs.iter().filter(|s| s.mi.is_some()).collect();
        assert_eq!(marks.len(), 2); // «token» в ошибке и в note
        assert_eq!(mi, 2);
    }

    #[test]
    fn path_extraction() {
        let v: serde_json::Value =
            serde_json::from_str(r#"{"data":{"items":[{"token":"abc"}]}}"#).unwrap();
        assert_eq!(extract_by_path(&v, "$.data.items.0.token").unwrap(), "abc");
        assert_eq!(
            extract_by_path(&v, "$.data").unwrap(),
            r#"{"items":[{"token":"abc"}]}"#
        );
        assert!(extract_by_path(&v, "$.nope").is_err());
    }

    #[test]
    fn vars_template() {
        let mut vars = HashMap::new();
        vars.insert("baseUrl".to_string(), "https://api.example.com".to_string());
        assert_eq!(
            tpl_global("{{baseUrl}}/x?q={{unknown}}", &vars),
            "https://api.example.com/x?q={{unknown}}"
        );
        let pv = url_preview("{{baseUrl}}/x?q={{unknown}}", &vars).unwrap();
        assert_eq!(
            pv[0],
            PreviewPart {
                text: "https://api.example.com".into(),
                ok: true
            }
        );
        assert_eq!(
            pv[1],
            PreviewPart {
                text: "/x?q=".into(),
                ok: true
            }
        );
        assert_eq!(
            pv[2],
            PreviewPart {
                text: "{{unknown}}".into(),
                ok: false
            }
        );
        assert!(url_preview("plain", &vars).is_none());
    }
}
