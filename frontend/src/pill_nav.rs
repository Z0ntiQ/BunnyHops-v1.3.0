//! «Tab Pill»: белая пилюля под активной вкладкой (дизайн DesignBookmark, 17.11.2026).
//! Два spring-объекта на пилюлю — по одному на край. Ведущий край жёсткий (k=620 c=40),
//! ведомый мягкий (k=170 c=19): пилюля тянется к новой вкладке и подтягивает хвост.
//! Движение — requestAnimationFrame, константы и шаг — как в оригинале Tab Pill.
use std::cell::RefCell;
use std::collections::HashMap;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

/// Spring из общего движка дизайна: x — текущее значение, t — цель, k — жёсткость, c — демпфирование.
struct Spring {
    x: f64,
    v: f64,
    t: f64,
    k: f64,
    c: f64,
}

impl Spring {
    fn new(x: f64) -> Self {
        Spring {
            x,
            v: 0.0,
            t: x,
            k: 620.0,
            c: 40.0,
        }
    }
    fn step(&mut self, dt: f64) {
        let a = -self.k * (self.x - self.t) - self.c * self.v;
        self.v += a * dt;
        self.x += self.v * dt;
    }
    fn done(&self) -> bool {
        (self.x - self.t).abs() < 0.3 && self.v.abs() < 0.5
    }
    fn snap(&mut self) {
        self.x = self.t;
        self.v = 0.0;
    }
    fn tune(&mut self, k: f64, c: f64) {
        self.k = k;
        self.c = c;
    }
}

struct Pill {
    a: Spring,
    b: Spring,
    idx: Option<usize>,
    running: bool,
    prev: f64,
}

thread_local! {
    static PILLS: RefCell<HashMap<String, Pill>> = RefCell::new(HashMap::new());
    // наблюдаем за треками и resize один раз на трек/окно — замыкания живут вечно
    static OBSERVED: RefCell<HashMap<String, web_sys::MutationObserver>> =
        RefCell::new(HashMap::new());
    static RESIZE_ON: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Индекс активной вкладки запроса по её id; None — вкладки нет (Body скрыт).
/// Порядок кнопок в .tab-track: params, headers, [body], steps, vars.
pub fn req_tab_index(t: &str, has_body: bool) -> Option<usize> {
    match t {
        "params" => Some(0),
        "headers" => Some(1),
        "body" => has_body.then_some(2),
        "steps" => Some(if has_body { 3 } else { 2 }),
        "vars" => Some(if has_body { 4 } else { 3 }),
        _ => Some(0),
    }
}

fn track_el(id: &str) -> Option<(web_sys::Element, web_sys::HtmlElement)> {
    let doc = web_sys::window()?.document()?;
    let track = doc
        .get_element_by_id(id)?
        .dyn_into::<web_sys::Element>()
        .ok()?;
    let pill = track
        .query_selector(".tab-pill")
        .ok()?
        .and_then(|p| p.dyn_into::<web_sys::HtmlElement>().ok())?;
    Some((track, pill))
}

/// (left, right) кнопки idx относительно трека: трок position:relative,
/// offsetLeft кнопки считается от него.
fn measure(track: &web_sys::Element, idx: usize) -> Option<(f64, f64)> {
    let btns = track.query_selector_all("button.tab").ok()?;
    let el = btns
        .item(idx as u32)?
        .dyn_into::<web_sys::HtmlElement>()
        .ok()?;
    let l = el.offset_left() as f64;
    Some((l, l + el.offset_width() as f64))
}

fn render_pill(pill: &web_sys::HtmlElement, a: f64, b: f64) {
    let st = pill.style();
    let _ = st.set_property("transform", &format!("translateX({:.1}px)", a));
    let _ = st.set_property("width", &format!("{:.1}px", (b - a).max(0.0)));
}

/// Один кадр: заново меряем цель (самокоррекция при смене ширины кнопок — счётчики
/// бейджей), шагаем пружины в 2 суб-шага, рисуем. true — нужен следующий кадр.
fn tick(id: &str, now: f64) -> bool {
    let Some((track, pill)) = track_el(id) else {
        drop_pill(id);
        return false;
    };
    PILLS.with(|pc| {
        let mut b = pc.borrow_mut();
        let Some(pl) = b.get_mut(id) else {
            return false;
        };
        if let Some(idx) = pl.idx {
            if let Some((l, r)) = measure(&track, idx) {
                pl.a.t = l;
                pl.b.t = r;
            }
        }
        let dt = ((now - pl.prev) / 1000.0).clamp(0.0, 1.0 / 30.0);
        pl.prev = now;
        for _ in 0..2 {
            pl.a.step(dt / 2.0);
            pl.b.step(dt / 2.0);
        }
        if pl.a.done() && pl.b.done() {
            pl.a.snap();
            pl.b.snap();
            render_pill(&pill, pl.a.x, pl.b.x);
            pl.running = false;
            false
        } else {
            render_pill(&pill, pl.a.x, pl.b.x);
            true
        }
    })
}

fn schedule(id: &str) {
    let Some(w) = web_sys::window() else { return };
    let id = id.to_string();
    let cb = Closure::once_into_js(move |now: f64| {
        if tick(&id, now) {
            schedule(&id);
        }
    });
    let _ = w.request_animation_frame(cb.unchecked_ref::<js_sys::Function>());
}

/// Разбудить пилюлю после изменения разметки трека (счётники бейджей, показ Body).
fn wake(id: &str) {
    let start = PILLS.with(|pc| {
        pc.borrow_mut()
            .get_mut(id)
            .map(|pl| {
                if pl.running {
                    false
                } else {
                    pl.running = true;
                    true
                }
            })
            .unwrap_or(false)
    });
    if start {
        schedule(id);
    }
}

/// Наблюдатель за треком + глобальный resize — один раз на трек.
fn ensure_reactive(id: &str, track: &web_sys::Element) {
    let known = OBSERVED.with(|o| o.borrow().contains_key(id));
    if !known {
        let id2 = id.to_string();
        let cb = Closure::wrap(Box::new(move |_: JsValue| {
            wake(&id2);
        }) as Box<dyn FnMut(JsValue)>);
        if let Ok(obs) =
            web_sys::MutationObserver::new(cb.as_ref().unchecked_ref::<js_sys::Function>())
        {
            std::mem::forget(cb);
            let init = web_sys::MutationObserverInit::new();
            init.set_child_list(true);
            init.set_subtree(true);
            init.set_character_data(true);
            let node: &web_sys::Node = track.unchecked_ref();
            let _ = obs.observe_with_options(node, &init);
            OBSERVED.with(|o| o.borrow_mut().insert(id.to_string(), obs));
        }
    }
    if !RESIZE_ON.with(|r| r.get()) {
        RESIZE_ON.with(|r| r.set(true));
        let cb = Closure::wrap(Box::new(move |_: web_sys::Event| {
            let ids: Vec<(String, Option<usize>)> = PILLS.with(|pc| {
                pc.borrow()
                    .iter()
                    .map(|(k, p)| (k.clone(), p.idx))
                    .collect()
            });
            for (id, idx) in ids {
                move_pill(&id, idx, false);
            }
        }) as Box<dyn FnMut(web_sys::Event)>);
        if let Some(w) = web_sys::window() {
            let _ = w.add_event_listener_with_callback("resize", cb.as_ref().unchecked_ref());
            cb.forget();
        }
    }
}

/// Снять пилюлю с учёта: трек размонтирован (mock-режим, пропажа ответа).
pub fn drop_pill(id: &str) {
    PILLS.with(|pc| pc.borrow_mut().remove(id));
    OBSERVED.with(|o| {
        if let Some(obs) = o.borrow_mut().remove(id) {
            obs.disconnect();
        }
    });
}

/// Поставить пилюлю под кнопку idx трека `id`; None — сжать в точку (вкладка скрыта).
/// animate=false — мгновенно; при первом появлении трека всегда мгновенно.
pub fn move_pill(id: &str, idx: Option<usize>, animate: bool) {
    let Some((track, pill)) = track_el(id) else {
        return;
    };
    let target = idx.and_then(|i| measure(&track, i));
    let new_entry = !PILLS.with(|pc| pc.borrow().contains_key(id));
    if new_entry {
        ensure_reactive(id, &track);
    }
    let start_anim = PILLS.with(|pc| {
        let mut b = pc.borrow_mut();
        let pl = b.entry(id.to_string()).or_insert_with(|| {
            let (l, r) = target.unwrap_or((0.0, 0.0));
            Pill {
                a: Spring::new(l),
                b: Spring::new(r),
                idx,
                running: false,
                prev: 0.0,
            }
        });
        let prev_idx = pl.idx;
        pl.idx = idx;
        match target {
            Some((l, r)) => {
                pl.a.t = l;
                pl.b.t = r;
            }
            // вкладка скрыта: сжаться к текущей левой кромке
            None => {
                pl.a.t = pl.a.x;
                pl.b.t = pl.a.x;
            }
        }
        if new_entry || !animate {
            pl.a.snap();
            pl.b.snap();
            render_pill(&pill, pl.a.x, pl.b.x);
            return false;
        }
        // направление хода: вправо — правый край ведёт, влево — левый
        let right = match (prev_idx, idx) {
            (Some(p), Some(n)) => n >= p,
            (None, Some(_)) => true,
            _ => true,
        };
        if right {
            pl.b.tune(620.0, 40.0);
            pl.a.tune(170.0, 19.0);
        } else {
            pl.a.tune(620.0, 40.0);
            pl.b.tune(170.0, 19.0);
        }
        if !pl.running {
            pl.running = true;
            true
        } else {
            false
        }
    });
    if start_anim {
        schedule(id);
    }
}
