// /api/eyes — снимает ТЕКУЩЕЕ окно приложения и отдаёт кадр.
// Чистая идемпотентная функция без привязки к внешним адресам: одинаковое
// состояние окна → одинаковый кадр. Куда кадр отправить (глаза, лог, отчёт) —
// решает вызывающий: кадр лежит в temp и путь возвращается, тот же кадр
// отдаётся телом ответа. Никаких адресов глаз здесь нет и не будет.
//
// Окно динамическое: границы читаются живым списком окон каждый вызов,
// ничего не фиксируется между вызовами.
//
// Параметры: json=1 (метаданные вместо PNG), full=1 (без масштабирования).
// По умолчанию — PNG bytes (image/png), широкие кадры режутся к 2060 по
// ширине — масштаб «разглядеть мелкий UI» из регламента прогонов.

use axum::extract::Query;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use std::collections::HashMap;

// Масштаб «разглядеть мелкий UI» — регламент прогонов: широкие кадры режем до 2060.
const MAX_W: u32 = 2060;

pub async fn eyes_handler(Query(q): Query<HashMap<String, String>>) -> Response {
    let full = q.contains_key("full");
    let frame = match capture_own_window(full) {
        Ok(f) => f,
        Err(e) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "ok": false, "error": e })),
            )
                .into_response()
        }
    };

    let png = match encode_png(&frame.rgba) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "ok": false, "error": e })),
            )
                .into_response()
        }
    };

    // кадр на диск: пусть остаётся доказательством, путь — в ответе
    let path = match save_frame(&png) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "ok": false, "error": e })),
            )
                .into_response()
        }
    };

    if q.contains_key("json") {
        return (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "path": path,
                "width": frame.width,
                "height": frame.height,
                "window_id": frame.window_id,
            })),
        )
            .into_response();
    }

    let mut headers = HeaderMap::new();
    headers.insert(
        "X-Frame-Path",
        HeaderValue::from_str(&path).unwrap_or_else(|_| HeaderValue::from_static("")),
    );
    (headers, [(axum::http::header::CONTENT_TYPE, "image/png")], png).into_response()
}

pub struct Frame {
    pub rgba: image::RgbaImage,
    pub width: u32,
    pub height: u32,
    pub window_id: u32,
}

fn encode_png(rgba: &image::RgbaImage) -> Result<Vec<u8>, String> {
    use image::ImageEncoder as _;
    let mut buf = std::io::Cursor::new(Vec::new());
    let enc = image::codecs::png::PngEncoder::new(&mut buf);
    enc.write_image(
        rgba.as_raw(),
        rgba.width(),
        rgba.height(),
        image::ExtendedColorType::Rgba8,
    )
    .map_err(|e| format!("png: {e}"))?;
    Ok(buf.into_inner())
}

fn save_frame(png: &[u8]) -> Result<String, String> {
    let dir = std::env::temp_dir().join("bunny-hops-eyes");
    std::fs::create_dir_all(&dir).map_err(|e| format!("temp dir: {e}"))?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = dir.join(format!("eyes-{ts}.png"));
    std::fs::write(&path, png).map_err(|e| format!("write: {e}"))?;
    Ok(path.to_string_lossy().to_string())
}

#[cfg(target_os = "macos")]
pub fn capture_own_window(full: bool) -> Result<Frame, String> {
    capture_own_window_impl(full)
}

#[cfg(target_os = "macos")]
fn capture_own_window_impl(full: bool) -> Result<Frame, String> {
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;
    use core_graphics::geometry::{CGPoint, CGRect, CGSize};
    use core_graphics::window::{
        copy_window_info, create_image, kCGNullWindowID, kCGWindowImageBestResolution,
        kCGWindowImageBoundsIgnoreFraming, kCGWindowListOptionIncludingWindow,
        kCGWindowListOptionOnScreenOnly,
    };

    // 1. Живой список окон: свои on-screen окна, layer 0, по площади.
    // Размер окна нигде не зашит — берёмся заново каждый вызов.
    let info = copy_window_info(kCGWindowListOptionOnScreenOnly, kCGNullWindowID)
        .ok_or("список окон недоступен")?;
    let my_pid = std::process::id() as i64;
    let key_pid = CFString::new("kCGWindowOwnerPID");
    let key_num = CFString::new("kCGWindowNumber");
    let key_layer = CFString::new("kCGWindowLayer");
    let key_bounds = CFString::new("kCGWindowBounds");
    let key_w = CFString::new("Width");
    let key_h = CFString::new("Height");
    let key_x = CFString::new("X");
    let key_y = CFString::new("Y");

    fn num(key: &CFString, dict: &CFDictionary<CFString, CFType>) -> Option<i64> {
        dict.find(key)
            .and_then(|v| v.downcast::<CFNumber>())
            .and_then(|n| n.to_i64())
    }

    fn real(key: &CFString, dict: &CFDictionary<CFString, CFType>) -> Option<f64> {
        dict.find(key)
            .and_then(|v| v.downcast::<CFNumber>())
            .and_then(|n| n.to_f64())
    }

    // (wid, area, rect) — rect берём из словаря границ: это живые границы окна,
    // которые меняются вместе с формой окна; ничего не фиксируется между вызовами.
    let mut candidates: Vec<(u32, u64, Option<CGRect>)> = Vec::new();
    for item in info.iter() {
        // записи списка окон документированы как словари — unsafe-обёртка валидна
        let d: CFDictionary<CFString, CFType> =
            unsafe { CFDictionary::wrap_under_get_rule(*item as *const _) };
        if num(&key_pid, &d) != Some(my_pid) {
            continue;
        }
        if num(&key_layer, &d).unwrap_or(-1) != 0 {
            continue;
        }
        let wid = num(&key_num, &d).unwrap_or(0) as u32;
        let mut rect = None;
        let area = d
            .find(&key_bounds)
            .map(|v| {
                let b: CFDictionary<CFString, CFType> = unsafe {
                    CFDictionary::wrap_under_get_rule(v.as_CFTypeRef() as *const _)
                };
                let w = real(&key_w, &b).unwrap_or(0.0).max(0.0);
                let h = real(&key_h, &b).unwrap_or(0.0).max(0.0);
                let x = real(&key_x, &b).unwrap_or(0.0);
                let y = real(&key_y, &b).unwrap_or(0.0);
                if w > 0.0 && h > 0.0 {
                    rect = Some(CGRect::new(&CGPoint::new(x, y), &CGSize::new(w, h)));
                }
                (w * h) as u64
            })
            .unwrap_or(0);
        if wid != 0 && area > 0 {
            candidates.push((wid, area, rect));
        }
    }
    candidates.sort_by_key(|(_, a, _)| std::cmp::Reverse(*a));
    if candidates.is_empty() {
        return Err("окно приложения не найдено (headless-режим?)".into());
    }

    // 2. Съёмка по кандидатам: пустой (чёрный) кадр — пробуем следующий.
    let mut last_err = "все кадры чёрные".to_string();
    for (wid, _, rect) in candidates {
        let rect = match rect {
            Some(r) => r,
            None => {
                last_err = format!("окно {wid}: нет границ");
                continue;
            }
        };
        let img = match create_image(
            rect,
            kCGWindowListOptionIncludingWindow,
            wid,
            kCGWindowImageBoundsIgnoreFraming | kCGWindowImageBestResolution,
        ) {
            Some(i) => i,
            None => {
                last_err = format!("окно {wid}: съёмка не удалась");
                continue;
            }
        };
        let w = img.width() as u32;
        let h = img.height() as u32;
        if w < 2 || h < 2 {
            last_err = format!("окно {wid}: кадр пустой");
            continue;
        }
        let bpr = img.bytes_per_row();
        let data = img.data();
        let src = data.bytes();
        // окно сервера отдаёт BGRA8; переводим в RGBA и проверяем, что кадр не чёрный
        let mut rgba = vec![0u8; (w as usize) * (h as usize) * 4];
        let mut lit = 0usize;
        for y in 0..h as usize {
            for x in 0..w as usize {
                let s = y * bpr + x * 4;
                if s + 3 >= src.len() {
                    break;
                }
                let b = src[s];
                let g = src[s + 1];
                let r = src[s + 2];
                let d = y * (w as usize) * 4 + x * 4;
                rgba[d] = r;
                rgba[d + 1] = g;
                rgba[d + 2] = b;
                rgba[d + 3] = 255;
                if (r as u32 + g as u32 + b as u32) > 30 {
                    lit += 1;
                }
            }
        }
        if lit * 200 < (w as usize * h as usize) {
            last_err = format!("окно {wid}: кадр чёрный");
            continue;
        }
        let mut out = image::RgbaImage::from_raw(w, h, rgba).ok_or("буфер кадра")?;
        // 3. Масштаб «разглядеть»: широкие кадры — к MAX_W по ширине (full=1 — как есть).
        if !full && out.width() > MAX_W {
            let nh = ((out.height() as u64 * MAX_W as u64) / out.width() as u64).max(1) as u32;
            out = image::imageops::resize(&out, MAX_W, nh, image::imageops::FilterType::Lanczos3);
        }
        return Ok(Frame {
            width: out.width(),
            height: out.height(),
            window_id: wid,
            rgba: out,
        });
    }
    Err(last_err)
}

#[cfg(not(target_os = "macos"))]
pub fn capture_own_window(_full: bool) -> Result<Frame, String> {
    Err("съёмка окна доступна только на macOS".into())
}
