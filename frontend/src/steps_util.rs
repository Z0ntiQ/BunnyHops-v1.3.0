//! steps_util: вынесено из app.rs (атомарные модули, правило пользователя 13.11.2026).

use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use crate::types::Step;

pub fn cloud_overflow() -> bool {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return false;
    };
    match doc.query_selector(".tab-cloud") {
        Ok(Some(el)) => el.scroll_width() > el.client_width() + 1,
        _ => false,
    }
}

/* ---------- small helpers for reactive step fields ---------- */

pub fn step_at(steps: &RwSignal<Vec<Step>>, i: usize, f: impl FnOnce(&Step) -> String) -> String {
    steps.get().get(i).map(f).unwrap_or_default()
}

pub fn step_at_opt<T: Clone>(steps: &RwSignal<Vec<Step>>, i: usize, f: impl FnOnce(&Step) -> T) -> Option<T> {
    steps.get().get(i).map(f)
}

pub fn update_step_at(steps: &RwSignal<Vec<Step>>, i: usize, f: impl FnOnce(&mut Step)) {
    steps.update(|v| { if let Some(s) = v.get_mut(i) { f(s); } });
}

pub fn update_step(steps: &RwSignal<Vec<Step>>, id: &str, f: impl FnOnce(&mut Step)) {
    steps.update(|v| {
        if let Some(s) = v.iter_mut().find(|s| s.id == id) {
            f(s);
        }
    });
}

