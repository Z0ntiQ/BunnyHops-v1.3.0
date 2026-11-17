use leptos::prelude::*;
use std::collections::HashMap;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

use crate::logic::*;
use crate::mock_view::ProxyView;
use crate::stream::{do_stream, err_message, is_abort_err, RespUpdate};
use crate::ui::*;


use crate::actlog::*;
use crate::method_select::*;
use crate::pill_nav;
use crate::search::*;
use crate::steps_util::*;
use crate::storage::*;
use crate::timefmt::*;
use crate::types::*;
use crate::util::*;

#[component]

pub fn App() -> impl IntoView {
    /* ---------- state (зеркало useState из App.jsx) ---------- */
    let method = create_rw_signal("GET".to_string());
    let search_q = create_rw_signal(String::new());
    let search_hits = create_rw_signal(0i64);
    /* ---------- поиск-кнопка (стиль command-button): пилюля + mod+K ---------- */
    let search_open = create_rw_signal(false);
    let held_mod = create_rw_signal(false);
    let held_k = create_rw_signal(false);
    let mac = web_sys::window()
        .and_then(|w| js_sys::Reflect::get(&JsValue::from(w.navigator()), &JsValue::from_str("platform")).ok())
        .and_then(|v| v.as_string())
        .unwrap_or_default()
        .to_lowercase()
        .contains("mac");
    let mod_label: &'static str = if mac { "⌘" } else { "Ctrl" };
    let close_search = move || {
        search_q.set(String::new());
        search_hits.set(0);
        hop_search("");
        search_open.set(false);
    };
    let url = create_rw_signal(String::new());
    // Живой плейсхолдер URL-поля (17.11.2026): подсказка печатается, когда поле пустое.
    let ph_hint = create_rw_signal(String::new());
    {
        fn sleep_ms(ms: i32) -> impl std::future::Future<Output = ()> {
            let promise = js_sys::Promise::new(&mut |resolve, _reject| {
                let _ = web_sys::window()
                    .expect("no window")
                    .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
            });
            async move {
                let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
            }
        }
        let hint = "https://ZontiQ.dev";
        wasm_bindgen_futures::spawn_local(async move {
            loop {
                for i in 0..=hint.chars().count() {
                    ph_hint.set(hint.chars().take(i).collect());
                    sleep_ms(90).await;
                }
                sleep_ms(20000).await;
                for i in (0..hint.chars().count()).rev() {
                    ph_hint.set(hint.chars().take(i).collect());
                    sleep_ms(36).await;
                }
                sleep_ms(700).await;
            }
        });
    }
    let params = create_rw_signal(Vec::<KvRow>::new());
    let headers = create_rw_signal(Vec::<KvRow>::new());
    let body = create_rw_signal(String::new());
    let tab = create_rw_signal("params".to_string());
    let mock_mode = create_rw_signal(false);
    let resp_tab = create_rw_signal("body".to_string());
    let pretty = create_rw_signal(true);
    let loading = create_rw_signal(false);
    let response = create_rw_signal(None::<RespState>);
    let error = create_rw_signal(None::<String>);
    let copied_req = create_rw_signal(false);
    let copied_resp = create_rw_signal(false);
    let scale_idx = create_rw_signal(load_scale_idx());
    let query = create_rw_signal(String::new());
    let match_idx = create_rw_signal(0usize);
    let steps = create_rw_signal(load_steps());
    let running = create_rw_signal(false);
    let open_step_id = create_rw_signal(None::<String>);
    let vars = create_rw_signal(load_vars());
    let saved = create_rw_signal(load_saved());
    let active_tab = create_rw_signal(None::<String>);
    let flash = create_rw_signal(None::<(String, u32)>);
    let del_confirm = create_rw_signal(None::<String>);
    let api_calls = create_rw_signal(Vec::<ApiCall>::new());
    let api_panel = create_rw_signal(false);
    // короткий лог действий в подвале: что упало и на каком шаге
    let act_log = create_rw_signal(Vec::<String>::new());
    let log_open = create_rw_signal(false);
    let log_h = create_rw_signal(
        ls_get(LS_LOG_H)
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(150),
    );
    let logit = move |msg: String| {
        let line = format!("{} {}", ts_short(), msg);
        act_log.update(|v| {
            v.push(line);
            if v.len() > 100 {
                let cut = v.len() - 100;
                v.drain(..cut);
            }
        });
        // единый логер: событие уходит и в серверный журнал (fire-and-forget)
        let payload = msg;
        spawn_local(async move {
            post_log(&payload).await;
        });
    };
    // панель журнала: серверный журнал — единый источник правды:
    // обновление при открытии и каждые 4 с, пока панель открыта.
    let _ = Effect::watch(
        move || log_open.get(),
        move |open, _, _| {
            if !*open {
                return;
            }
            spawn_local(async move {
                loop {
                    let lines = fetch_action_log().await;
                    if !lines.is_empty() {
                        act_log.set(lines);
                    }
                    sleep_ms_local(4000).await;
                    if !log_open.get_untracked() {
                        break;
                    }
                }
            });
        },
        false,
    );
    let groups = create_rw_signal(load_groups());
    let group_modal = create_rw_signal(false);
    let group_name = create_rw_signal(String::new());
    let modal_open = create_rw_signal(false);
    let modal_name = create_rw_signal(String::new());
    let modal_err = create_rw_signal(String::new());
    let cloud_full = create_rw_signal(false);
    let abort_ctrl = create_rw_signal(None::<web_sys::AbortController>);
    let run_abort_ctrl = create_rw_signal(None::<web_sys::AbortController>);
    let timer_id = create_rw_signal(None::<i32>);

    /* ---------- persistence ---------- */
    let _ = Effect::watch(
        move || serde_json::to_string(&steps.get()).unwrap_or_default(),
        move |ser, _, _| {
            // пустой список не пишем, чтобы не затирать сохранённые сценарии (как в оригинале)
            if serde_json::from_str::<Vec<Step>>(ser).map(|v| !v.is_empty()).unwrap_or(false) {
                ls_set(LS_STEPS, ser);
            }
        },
        false,
    );
    let _ = Effect::watch(
        move || serde_json::to_string(&vars.get()).unwrap_or_default(),
        move |ser, _, _| ls_set(LS_VARS, ser),
        false,
    );
    let _ = Effect::watch(
        move || serde_json::to_string(&saved.get()).unwrap_or_default(),
        move |ser, _, _| ls_set(LS_SAVED, ser),
        false,
    );
    let _ = Effect::watch(
        move || serde_json::to_string(&groups.get()).unwrap_or_default(),
        move |ser, _, _| ls_set(LS_GROUPS, ser),
        false,
    );
    let _ = Effect::watch(
        move || scale_idx.get(),
        move |v, _, _| ls_set(LS_SCALE_IDX, &v.to_string()),
        false,
    );

    /* ---------- derived ---------- */
    let enabled_params =
        Memo::new(move |_| params.get().iter().filter(|p| p.on && !p.key.trim().is_empty()).count());
    let vars_map = Memo::new(move |_| {
        let mut m = HashMap::new();
        for v in vars.get() {
            let k = v.key.trim().to_string();
            if v.on && !k.is_empty() {
                m.insert(k, v.value);
            }
        }
        m
    });
    let has_body = Memo::new(move |_| !is_bodyless(&method.get()));
    let can_send =
        Memo::new(move |_| !url.get().trim().is_empty() && !loading.get() && !running.get());

    /* ---------- url <-> params ---------- */
    let on_params_change = Callback::new(move |rows: Vec<KvRow>| {
        params.set(rows.clone());
        let base = split_url(&url.get_untracked()).base;
        let qs = params_to_query(&rows);
        url.set(if qs.is_empty() { base } else { format!("{base}?{qs}") });
    });
    let on_url_change = Callback::new(move |v: String| {
        url.set(v.clone());
        params.set(query_to_params(&split_url(&v).query));
    });

    /* ---------- saved requests ---------- */
    let save_current = Callback::new(move |_: ()| {
        let u = url.get_untracked();
        if u.trim().is_empty() {
            return;
        }
        let m = method.get_untracked();
        let h = headers.get_untracked();
        let b = body.get_untracked();
        let typed = modal_name.get_untracked().trim().to_string();
        let snap = response.get_untracked().unwrap_or(RespState {
            status: 0,
            status_text: String::new(),
            headers: Vec::new(),
            content_type: String::new(),
            final_url: String::new(),
            body_text: String::new(),
            bytes: 0,
            time_ms: 0.0,
            started_at: 0.0,
            is_textual: false,
            streaming: false,
            cancelled: false,
            image_blob_url: None,
        });
        let resp_body = if snap.is_textual { snap.body_text.clone() } else { String::new() };
        let mut list = saved.get_untracked();
        // редактор привязан к открытой вкладке и нового имени не ввели:
        // явное сохранение обновляет эту вкладку на месте
        if typed.is_empty() {
            if let Some(id) = active_tab.get_untracked() {
                if let Some(i) = list.iter().position(|r| r.id == id) {
                    let updated = SavedReq {
                        id: id.clone(),
                        name: list[i].name.clone(),
                        method: m.clone(),
                        url: u.clone(),
                        headers: h.clone(),
                        body: b.clone(),
                        resp_status: snap.status,
                        resp_body,
                    };
                    list[i] = updated;
                    saved.set(list);
                    modal_open.set(false);
                    return;
                }
            }
        }
        // новая вкладка: занятое имя — отдельная ошибка
        if !typed.is_empty() && list.iter().any(|r| r.name == typed) {
            modal_err.set("\u{26a0} Вкладка с таким именем уже есть — выбери другое.".to_string());
            return;
        }
        // дубликат: метод+url+заголовки+тело совпадают с уже сохранённой вкладкой
        if let Some(r) = list
            .iter()
            .find(|r| r.method == m && r.url == u && r.headers == h && r.body == b)
        {
            modal_err.set(format!(
                "\u{26a0} Запрос полностью совпадает с вкладкой «{}» — тот же метод, URL, заголовки и тело.",
                r.name
            ));
            return;
        }
        modal_err.set(String::new());
        let name = if !typed.is_empty() {
            typed
        } else {
            let base = split_url(&u).base;
            let name = base
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .trim_end_matches('/')
                .to_string();
            if name.is_empty() { base } else { name }
        };
        let item = SavedReq {
            id: new_id(),
            name,
            method: m,
            url: u,
            headers: h,
            body: b,
            resp_status: snap.status,
            resp_body,
        };
        logit(format!("\u{ff0b} \u{432}\u{43a}\u{43b}\u{430}\u{434}\u{43a}\u{430} \u{00ab}{}\u{00bb} \u{441}\u{43e}\u{445}\u{440}\u{430}\u{43d}\u{435}\u{43d}\u{430}", item.name));
        let new_id_ = item.id.clone();
        list.insert(0, item);
        saved.set(list);
        active_tab.set(Some(new_id_));
        modal_open.set(false);
        modal_name.set(String::new());
    });
    let open_modal = Callback::new(move |_: ()| {
        if cloud_full.get_untracked() {
            let num = groups
                .get_untracked()
                .iter()
                .map(|g| g.num)
                .max()
                .unwrap_or(0)
                + 1;
            group_name.set(format!("Группа {num}"));
            group_modal.set(true);
            return;
        }
        if url.get_untracked().trim().is_empty() {
            return;
        }
        modal_name.set(String::new());
        modal_err.set(String::new());
        modal_open.set(true);
    });
    // приглашение сохранить: есть живой запрос и ни одна вкладка ему не равна
    let invite_save = create_memo(move |_| {
        let u = url.get();
        if u.trim().is_empty() {
            return false;
        }
        let m = method.get();
        let h = headers.get();
        let b = body.get();
        !saved.get().into_iter().any(|r| {
            r.method == m && r.url == u && r.headers == h && r.body == b
        })
    });
    let load_saved_req = Callback::new(move |id: String| {
        if let Some(r) = saved.get_untracked().into_iter().find(|r| r.id == id) {
            logit(format!("\u{25c7} \u{432}\u{43a}\u{43b}\u{430}\u{434}\u{43a}\u{430} \u{00ab}{}\u{00bb} \u{437}\u{430}\u{433}\u{440}\u{443}\u{436}\u{435}\u{43d}\u{430}", r.name));
            active_tab.set(Some(id.clone()));
            flash.set(None);
            method.set(r.method.clone());
            on_url_change.run(r.url.clone());
            headers.set(r.headers.clone());
            body.set(r.body.clone());
            if r.resp_status > 0 || !r.resp_body.is_empty() {
                let bytes = r.resp_body.len() as u64;
                response.set(Some(RespState {
                    status: r.resp_status,
                    status_text: String::new(),
                    headers: Vec::new(),
                    content_type: String::new(),
                    final_url: r.url.clone(),
                    body_text: r.resp_body.clone(),
                    bytes,
                    time_ms: 0.0,
                    started_at: 0.0,
                    is_textual: !r.resp_body.is_empty(),
                    streaming: false,
                    cancelled: false,
                    image_blob_url: None,
                }));
            } else {
                response.set(None);
            }
        }
    });
    let delete_saved = Callback::new(move |id: String| {
        if active_tab.get_untracked().as_deref() == Some(id.as_str()) {
            active_tab.set(None);
        }
        logit("\u{2715} \u{432}\u{43a}\u{43b}\u{430}\u{434}\u{43a}\u{430} \u{443}\u{434}\u{430}\u{43b}\u{435}\u{43d}\u{430}".to_string());
        let mut list = saved.get_untracked();
        list.retain(|r| r.id != id);
        saved.set(list);
    });

    let save_group = Callback::new(move |_: ()| {
        let num = groups
            .get_untracked()
            .iter()
            .map(|g| g.num)
            .max()
            .unwrap_or(0)
            + 1;
        let g = SavedGroup {
            id: new_id(),
            num,
            name: group_name.get_untracked(),
            tabs: saved.get_untracked(),
        };
        logit(format!("\u{25a3} \u{433}\u{440}\u{443}\u{43f}\u{43f}\u{430} {} \u{441}\u{43e}\u{437}\u{434}\u{430}\u{43d}\u{430} ({} \u{432}\u{43a}\u{43b}\u{430}\u{434})", g.num, g.tabs.len()));
        let mut list = groups.get_untracked();
        list.push(g);
        groups.set(list);
        saved.set(Vec::new());
        active_tab.set(None);
        group_modal.set(false);
    });
    let restore_group = Callback::new(move |id: String| {
        if let Some(g) = groups.get_untracked().into_iter().find(|g| g.id == id) {
            logit(format!("\u{25a3} \u{433}\u{440}\u{443}\u{43f}\u{43f}\u{430} {} \u{432}\u{435}\u{440}\u{43d}\u{443}\u{442}\u{430} \u{432} \u{43e}\u{431}\u{43b}\u{430}\u{43a}\u{43e} ({} \u{432}\u{43a}\u{43b}\u{430}\u{434})", g.num, g.tabs.len()));
            saved.set(g.tabs);
            let mut list = groups.get_untracked();
            list.retain(|x| x.id != id);
            groups.set(list);
        }
    });

    let refresh_api = Callback::new(move |_: ()| {
        let api_calls = api_calls;
        spawn_local(async move {
            api_calls.set(fetch_api_calls().await);
        });
    });
    let load_api_call = Callback::new(move |c: ApiCall| {
        logit(format!("\u{25c7} \u{432}\u{44b}\u{437}\u{43e}\u{432} API: {} {}", c.method, c.url));
        api_panel.set(false);
        active_tab.set(None);
        method.set(c.method.clone());
        on_url_change.run(c.url.clone());
        headers.set(c.headers.clone());
        body.set(c.body.clone());
        response.set(None);
    });
    refresh_api.run(());

    /* автосохранение ответа: отклик пишется в активную вкладку, чип вспыхивает.
       Запрос (метод/URL/заголовки/тело) во вкладку не пишем: иначе любое
       правка после загрузки вкладки молча перезаписала бы сохранённый запрос,
       а «сохранить как новую» всегда давало бы ложный дубликат. */
    let _ = Effect::watch(
        move || {
            let resp = response.get();
            resp.map(|r| (r.status, r.is_textual, r.body_text.clone(), r.streaming))
        },
        move |st, prev, _| {
            if prev.is_none() {
                return;
            }
            let Some(id) = active_tab.get_untracked() else { return };
            let (status, is_textual, body_text, streaming) =
                st.clone().unwrap_or((0, false, String::new(), false));
            if streaming {
                return;
            }
            let mut list = saved.get_untracked();
            let Some(i) = list.iter().position(|r| r.id == id) else {
                active_tab.set(None);
                return;
            };
            let updated = SavedReq {
                resp_status: status,
                resp_body: if is_textual { body_text } else { String::new() },
                ..list[i].clone()
            };
            if updated == list[i] {
                return;
            }
            let token = flash.get_untracked().map(|(_, t)| t).unwrap_or(0) + 1;
            list[i] = updated;
            saved.set(list);
            flash.set(Some((id, token)));
        },
        false,
    );

    let _ = Effect::watch(
        move || saved.get().len(),
        move |_, _, _: Option<()>| {
            let cf = cloud_full;
            let cb = Closure::wrap(Box::new(move || {
                cf.set(cloud_overflow());
            }) as Box<dyn FnMut()>);
            if let Some(w) = web_sys::window() {
                let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(
                    cb.as_ref().unchecked_ref(),
                    80,
                );
            }
            cb.forget();
        },
        false,
    );
    {
        let cf = cloud_full;
        let on_resize = Closure::wrap(Box::new(move || {
            cf.set(cloud_overflow());
        }) as Box<dyn FnMut()>);
        if let Some(w) = web_sys::window() {
            let _ = w.add_event_listener_with_callback("resize", on_resize.as_ref().unchecked_ref());
        }
        on_resize.forget();
    }

    /* ---------- copy ---------- */
    let mark_copied = |copied: RwSignal<bool>| Callback::new(move |_: ()| {
        copied.set(true);
        let cb = Closure::wrap(Box::new(move || copied.set(false)) as Box<dyn FnMut()>);
        if let Some(w) = web_sys::window() {
            let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(
                cb.as_ref().unchecked_ref(),
                1200,
            );
        }
        cb.forget();
    });

    /* ---------- send / stop ---------- */
    let send = Callback::new(move |_: ()| {
        let target = tpl_global(url.get_untracked().trim(), &vars_map.get_untracked());
        if target.is_empty() || loading.get_untracked() || running.get_untracked() {
            return;
        }
        let ctrl = web_sys::AbortController::new().unwrap();
        abort_ctrl.set(Some(ctrl.clone()));
        loading.set(true);
        error.set(None);
        let vm = vars_map.get_untracked();
        let h: Vec<(String, String)> = headers
            .get_untracked()
            .iter()
            .filter(|p| p.on && !p.key.trim().is_empty())
            .map(|p| (tpl_global(p.key.trim(), &vm), tpl_global(&p.value, &vm)))
            .collect();
        let body_t = tpl_global(&body.get_untracked(), &vm);
        let m = method.get_untracked();
        logit(format!("\u{2192} {m} {target}"));
        let signal = ctrl.signal().clone();
        spawn_local(async move {
            let head = move |u: RespUpdate| {
                response.set(Some(RespState {
                    status: u.status,
                    status_text: u.status_text,
                    headers: u.headers,
                    content_type: u.content_type,
                    final_url: u.final_url,
                    body_text: String::new(),
                    bytes: 0,
                    time_ms: 0.0,
                    started_at: js_now(),
                    is_textual: u.is_textual,
                    streaming: true,
                    cancelled: false,
                    image_blob_url: None,
                }));
                resp_tab.set("body".to_string());
            };
            // Троттлинг: large-ответы приходят десятками чанков, а перерисовка
            // тела на каждом чанке — O(чанки × размер) и вешает вкладку.
            let paint_at = create_rw_signal(0f64);
            let on_chunk = move |bytes: u64, text: String| {
                let now = js_now();
                if now - paint_at.get_untracked() < 150.0 {
                    return;
                }
                paint_at.set(now);
                response.update(|r| {
                    if let Some(resp) = r.as_mut() {
                        resp.bytes = bytes;
                        resp.body_text = text;
                    }
                });
            };
            match do_stream(&target, &m, &h, &body_t, &signal, head, on_chunk).await {
                Ok(out) => {
                    if out.network_error {
                        let t: String = out.text.chars().take(120).collect();
                        error.set(Some(out.text));
                        logit(format!("\u{2715} {t}"));
                    } else {
                        logit(format!("\u{2190} {} \u{00b7} {:.0} ms \u{00b7} {} \u{411}", out.status, out.time_ms, out.bytes));
                        response.update(|r| {
                            if let Some(resp) = r.as_mut() {
                                resp.bytes = out.bytes;
                                resp.body_text = out.text;
                                resp.time_ms = out.time_ms;
                                resp.streaming = false;
                                resp.image_blob_url = out.image_blob_url;
                            }
                        });
                    }
                }
                Err(e) => {
                    if is_abort_err(&e) {
                        logit("\u{23f9} \u{43e}\u{441}\u{442}\u{430}\u{43d}\u{43e}\u{432}\u{43b}\u{435}\u{43d}\u{43e}".to_string());
                        response.update(|r| match r.as_mut() {
                            Some(resp) => {
                                resp.streaming = false;
                                resp.cancelled = true;
                            }
                            None => {
                                *r = None;
                                error.set(Some("⏹ Запрос остановлен".to_string()));
                            }
                        });
                    } else {
                        let t = err_message(&e);
                        error.set(Some(t.clone()));
                        logit(format!("\u{2715} {}", t.chars().take(120).collect::<String>()));
                    }
                }
            }
            loading.set(false);
            abort_ctrl.set(None);
        });
    });
    let stop = Callback::new(move |_: ()| {
        if let Some(c) = abort_ctrl.get_untracked() {
            c.abort();
        }
        if let Some(c) = run_abort_ctrl.get_untracked() {
            c.abort();
        }
    });

    /* ---------- live timer while streaming ---------- */
    let _ = Effect::watch(
        move || response.get().map(|r| r.streaming).unwrap_or(false),
        move |&streaming, _, _| {
            if let Some(id) = timer_id.get_untracked() {
                if let Some(w) = web_sys::window() {
                    w.clear_interval_with_handle(id);
                }
                timer_id.set(None);
            }
            if !streaming {
                return;
            }
            let cb = Closure::wrap(Box::new(move || {
                response.update(|r| {
                    if let Some(resp) = r.as_mut() {
                        if resp.streaming {
                            resp.time_ms = js_now() - resp.started_at;
                        }
                    }
                });
            }) as Box<dyn FnMut()>);
            if let Some(w) = web_sys::window() {
                if let Ok(id) = w.set_interval_with_callback_and_timeout_and_arguments_0(
                    cb.as_ref().unchecked_ref(),
                    100,
                ) {
                    timer_id.set(Some(id));
                }
            }
            cb.forget();
        },
        false,
    );

    /* ---------- scenario steps ---------- */
    let add_step_from_current = Callback::new(move |_: ()| {
        let u = url.get_untracked().trim().to_string();
        if u.is_empty() {
            return;
        }
        let s = Step {
            id: new_id(),
            method: method.get_untracked(),
            url: u,
            body: body.get_untracked(),
            headers: headers.get_untracked(),
            capture_name: String::new(),
            capture_path: "$.token".to_string(),
            status: "idle".to_string(),
            result: None,
        };
        steps.update(|v| v.push(s.clone()));
        open_step_id.set(Some(s.id.clone()));
        tab.set("steps".to_string());
    });
    let remove_step = Callback::new(move |id: String| {
        steps.update(|v| v.retain(|s| s.id != id));
        open_step_id.update(|o| {
            if o.as_deref() == Some(id.as_str()) {
                *o = None;
            }
        });
    });
    let clear_steps = Callback::new(move |_: ()| {
        steps.set(Vec::new());
        ls_del(LS_STEPS);
    });
    let reset_steps = Callback::new(move |_: ()| {
        steps.update(|v| {
            for s in v.iter_mut() {
                s.status = "idle".to_string();
                s.result = None;
            }
        });
    });

    let run_all = Callback::new(move |_: ()| {
        if running.get_untracked() || steps.get_untracked().is_empty() {
            return;
        }
        running.set(true);
        pretty.set(true);
        let ctrl = web_sys::AbortController::new().unwrap();
        run_abort_ctrl.set(Some(ctrl.clone()));
        steps.update(|v| {
            for s in v.iter_mut() {
                s.status = "idle".to_string();
                s.result = None;
            }
        });
        spawn_local(async move {
            let ids: Vec<String> = steps.get_untracked().iter().map(|s| s.id.clone()).collect();
            let mut captured: HashMap<String, String> = HashMap::new();
            let mut aborted = false;
            for (i, id) in ids.iter().enumerate() {
                if ctrl.signal().aborted() {
                    aborted = true;
                    break;
                }
                let Some(step) = steps.get_untracked().iter().find(|s| s.id == *id).cloned() else {
                    continue;
                };
                open_step_id.set(Some(id.clone()));
                let merged = {
                    let mut m = vars_map.get_untracked();
                    for (k, v) in captured.iter() {
                        m.insert(k.clone(), v.clone());
                    }
                    m
                };
                let target = tpl_global(&step.url, &merged).trim().to_string();
                if target.is_empty() {
                    update_step(&steps, id, |s| {
                        s.status = "fail".into();
                        s.result = Some(StepResult {
                            status: StepStatusVal::Text("✗ пустой URL".into()),
                            time_ms: 0.0,
                            bytes: 0,
                            capture_note: String::new(),
                        });
                    });
                    continue;
                }
                let h: Vec<(String, String)> = step
                    .headers
                    .iter()
                    .filter(|p| p.on && !p.key.trim().is_empty())
                    .map(|p| {
                        (
                            tpl_global(p.key.trim(), &merged),
                            tpl_global(&p.value, &merged),
                        )
                    })
                    .collect();
                let body_t = tpl_global(&step.body, &merged);
                logit(format!("\u{25b6} \u{448}\u{430}\u{433} {}: {} {}", i + 1, step.method, target));
                let signal = ctrl.signal().clone();
                let res = do_stream(
                    &target,
                    &step.method,
                    &h,
                    &body_t,
                    &signal,
                    |_| {},
                    |_, _| {},
                )
                .await;
                match res {
                    Ok(out) if out.network_error => {
                        logit(format!("\u{2717} \u{448}\u{430}\u{433} {}: \u{441}\u{435}\u{442}\u{44c}", i + 1));
                        update_step(&steps, id, |s| {
                            s.status = "fail".into();
                            s.result = Some(StepResult {
                                status: StepStatusVal::Text("✗ сеть".into()),
                                time_ms: 0.0,
                                bytes: 0,
                                capture_note: String::new(),
                            });
                        });
                    }
                    Ok(out) => {
                        logit(format!("\u{2190} \u{448}\u{430}\u{433} {}: {} \u{00b7} {:.0} ms", i + 1, out.status, out.time_ms));
                        let ok = out.status < 400;
                        let mut capture_note = String::new();
                        let mut captured_val: Option<(String, String)> = None;
                        if !step.capture_name.is_empty() && ok {
                            match serde_json::from_str::<serde_json::Value>(&out.text) {
                                Ok(v) => {
                                    let path =
                                        if step.capture_path.is_empty() { "$" } else { &step.capture_path };
                                    match extract_by_path(&v, path) {
                                        Ok(val) => {
                                            let short: String =
                                                val.chars().take(24).collect();
                                            capture_note =
                                                format!(" {}:={}", step.capture_name, short);
                                            captured_val = Some((step.capture_name.clone(), val));
                                        }
                                        Err(e) => {
                                            capture_note =
                                                format!(" {}: {}", step.capture_name, e);
                                        }
                                    }
                                }
                                Err(e) => {
                                    capture_note =
                                        format!(" {}: {}", step.capture_name, e);
                                }
                            }
                        }
                        update_step(&steps, id, |s| {
                            s.status = if ok { "ok" } else { "fail" }.into();
                            s.result = Some(StepResult {
                                status: StepStatusVal::Code(out.status),
                                time_ms: out.time_ms,
                                bytes: out.bytes,
                                capture_note: capture_note.clone(),
                            });
                        });
                        if let Some((name, val)) = captured_val {
                            captured.insert(name, val);
                        }
                    }
                    Err(e) if is_abort_err(&e) => {
                        aborted = true;
                        break;
                    }
                    Err(e) => {
                        error.set(Some(err_message(&e)));
                        break;
                    }
                }
            }
            if aborted {
                steps.update(|v| {
                    for s in v.iter_mut() {
                        if s.status == "running" {
                            s.status = "fail".into();
                            s.result = Some(StepResult {
                                status: StepStatusVal::Text("⏹".into()),
                                time_ms: 0.0,
                                bytes: 0,
                                capture_note: String::new(),
                            });
                        }
                    }
                });
            }
            steps.update(|v| {
                for s in v.iter_mut() {
                    if s.status == "idle" {
                        s.status = "skipped".into();
                    }
                }
            });
            running.set(false);
            run_abort_ctrl.set(None);
        });
    });

    /* ---------- search ---------- */
    let _ = Effect::watch(
        move || query.get(),
        move |_, _, _| {
            match_idx.set(0);
        },
        false,
    );
    let search = Memo::new(move |_| {
        let resp = response.get();
        let Some(resp) = resp.as_ref().filter(|r| r.is_textual) else {
            return None;
        };
        let shown: String = if resp.streaming {
            resp.body_text.clone()
        } else {
            pretty
                .get()
                .then(|| pretty_json(&resp.body_text))
                .flatten()
                .unwrap_or_else(|| resp.body_text.clone())
        };
        let shown: String = shown.chars().take(200_000).collect();
        let mut total = 0usize;
        let lines: Vec<LineData> = shown
            .split('\n')
            .map(|t| {
                let is_json = resp.content_type.contains("json");
                let segs = if is_json { build_segments } else { plain_segments }(t, &query.get(), &mut || {
                    let v = total;
                    total += 1;
                    v
                });
                LineData {
                    hit: segs.iter().any(|s| s.mi.is_some()),
                    has_error: segs.iter().any(|s| s.err),
                    segs,
                }
            })
            .collect();
        Some(SearchData { total, lines })
    });
    let match_count = Memo::new(move |_| search.get().map(|s| s.total).unwrap_or(0));
    let cur_match = Memo::new(move |_| {
        let mc = match_count.get();
        if mc == 0 {
            return 0;
        }
        (match_idx.get() % mc + mc) % mc
    });
    let goto_match = move |delta: i64| {
        let mc = match_count.get_untracked();
        if mc == 0 {
            return;
        }
        let mi = match_idx.get_untracked() as i64;
        match_idx.set((((mi + delta) % mc as i64 + mc as i64) % mc as i64) as usize);
    };
    // прокрутка к текущему совпадению
    let _ = Effect::watch(
        move || (cur_match.get(), search.get().map(|_| ())),
        move |_, _, _| {
            let cur = cur_match.get_untracked();
            if match_count.get_untracked() == 0 {
                return;
            }
            let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
                return;
            };
            let Some(el) = doc
                .query_selector(&format!("[data-mi=\"{cur}\"]"))
                .ok()
                .flatten()
            else {
                return;
            };
            let opts = web_sys::ScrollIntoViewOptions::new();
            opts.set_behavior(web_sys::ScrollBehavior::Auto);
            opts.set_block(web_sys::ScrollLogicalPosition::Center);
            el.scroll_into_view_with_scroll_into_view_options(&opts);
        },
        false,
    );
    // автопрокрутка к активному шагу
    let _ = Effect::watch(
        move || {
            steps
                .get()
                .iter()
                .find(|s| s.status == "running")
                .map(|s| s.id.clone())
        },
        move |_, _, _| {
            let Some(id) = steps
                .get_untracked()
                .iter()
                .find(|s| s.status == "running")
                .map(|s| s.id.clone())
            else {
                return;
            };
            let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
                return;
            };
            let Some(el) = doc.query_selector(&format!("[data-step=\"{id}\"]")).ok().flatten() else {
                return;
            };
            let opts = web_sys::ScrollIntoViewOptions::new();
            opts.set_behavior(web_sys::ScrollBehavior::Smooth);
            opts.set_block(web_sys::ScrollLogicalPosition::Nearest);
            el.scroll_into_view_with_scroll_into_view_options(&opts);
        },
        false,
    );

    /* ---------- window hotkeys: Ctrl+F, Esc ---------- */
    if let Some(win) = web_sys::window() {
        let closure = Closure::<dyn FnMut(web_sys::KeyboardEvent)>::wrap(Box::new(move |ev: web_sys::KeyboardEvent| {

        let ctrl = ev.ctrl_key() || ev.meta_key();
        if ctrl && ev.key().to_lowercase() == "f" {
            ev.prevent_default();
            if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                if let Some(el) = doc.query_selector("#hop-search-input").ok().flatten() {
                    let _ = el.dyn_ref::<web_sys::HtmlElement>().map(|el| el.focus());
                }
            }
            return;
        }
        if ev.key() == "Escape" {
            if api_panel.get_untracked() {
                api_panel.set(false);
                return;
            }
            let focused_search = web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.active_element())
                .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok())
                .map(|el| el.id() == "hop-search-input")
                .unwrap_or(false);
            if focused_search {
                query.set(String::new());
                if let Some(el) = web_sys::window()
                    .and_then(|w| w.document())
                    .and_then(|d| d.query_selector("#hop-search-input").ok())
                    .flatten()
                {
                    let _ = el.dyn_ref::<web_sys::HtmlElement>().map(|el| el.blur());
                }
            }
        }
            }) as Box<dyn FnMut(web_sys::KeyboardEvent)>);
        let _ = win.add_event_listener_with_callback("keydown", closure.as_ref().unchecked_ref());
        closure.forget();
    }

    /* ---------- поиск-кнопка: mod+K, подсветка клавиш, сворачивание по клику вне ---------- */
    if let Some(win) = web_sys::window() {
        let so = search_open;
        let kd = Closure::<dyn FnMut(web_sys::KeyboardEvent)>::wrap(Box::new(move |ev: web_sys::KeyboardEvent| {
            if so.get_untracked() {
                return;
            }
            let m = ev.ctrl_key() || ev.meta_key();
            let k = ev.key().to_lowercase() == "k";
            held_mod.set(m);
            held_k.set(k);
            if m && k && !ev.repeat() {
                ev.prevent_default();
                so.set(true);
            }
        }) as Box<dyn FnMut(web_sys::KeyboardEvent)>);
        let _ = win.add_event_listener_with_callback("keydown", kd.as_ref().unchecked_ref());
        kd.forget();
        let ku = Closure::<dyn FnMut(web_sys::KeyboardEvent)>::wrap(Box::new(move |_: web_sys::KeyboardEvent| {
            held_mod.set(false);
            held_k.set(false);
        }) as Box<dyn FnMut(web_sys::KeyboardEvent)>);
        let _ = win.add_event_listener_with_callback("keyup", ku.as_ref().unchecked_ref());
        ku.forget();
        let cs = close_search;
        let pd = Closure::<dyn FnMut(web_sys::PointerEvent)>::wrap(Box::new(move |ev: web_sys::PointerEvent| {
            if !search_open.get_untracked() {
                return;
            }
            let inside = ev.target()
                .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                .and_then(|el| el.closest(".cmd-search").ok().flatten())
                .is_some();
            if !inside {
                cs();
            }
        }) as Box<dyn FnMut(web_sys::PointerEvent)>);
        let _ = win.add_event_listener_with_callback("pointerdown", pd.as_ref().unchecked_ref());
        pd.forget();
    }
    create_effect(move |_| {
        if search_open.get() {
            if let Some(el) = web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.query_selector("#hop-screen-search").ok())
                .flatten()
            {
                let _ = el.dyn_ref::<web_sys::HtmlElement>().map(|el| el.focus());
            }
        }
    });

    let zoom_by = move |delta: i64| {
        let next = (scale_idx.get_untracked() as i64 + delta).clamp(0, SCALES.len() as i64 - 1);
        scale_idx.set(next as usize);
    };

    // Полный адрес сервера в заголовке окна: document.title Tauri транслирует в title-bar (правило пользователя 17.11.2026).
    create_effect(move |_| {
        let Some(win) = web_sys::window() else { return };
        let host = win.location().host().unwrap_or_default();
        if let Some(doc) = win.document() {
            doc.set_title(&format!("Bunny Hops \u{2014} http://{host}"));
        }
    });

    // Пилюля под активной вкладкой («Tab Pill», дизайн 17.11.2026): эффект пересобирает
    // цель при смене вкладки, показе/скрытии вкладки Body и монтировании/размонтировании трека.
    let req_track = create_node_ref::<leptos::html::Div>();
    let resp_track = create_node_ref::<leptos::html::Div>();
    let mut req_snap = true;
    create_effect(move |_| {
        let t = tab.get();
        let hb = has_body.get();
        if req_track.get().is_none() {
            pill_nav::drop_pill("req-tabs");
            req_snap = true;
            return;
        }
        pill_nav::move_pill("req-tabs", pill_nav::req_tab_index(&t, hb), !req_snap);
        req_snap = false;
    });
    let mut resp_snap = true;
    create_effect(move |_| {
        let t = resp_tab.get();
        if resp_track.get().is_none() {
            pill_nav::drop_pill("resp-tabs");
            resp_snap = true;
            return;
        }
        pill_nav::move_pill(
            "resp-tabs",
            Some(if t == "headers" { 1 } else { 0 }),
            !resp_snap,
        );
        resp_snap = false;
    });

    /* ---------- render (1:1 с App.jsx) ---------- */
    let url_preview_m = Memo::new(move |_| url_preview(&url.get(), &vars_map.get()));
    let enabled_headers = Memo::new(move |_| {
        headers.get().iter().filter(|h| h.on && !h.key.trim().is_empty()).count()
    });

    view! {
        <div class="app" style=move || format!("--ui-scale: {}", SCALES[scale_idx.get()])>
            <header class="topbar">
                <div class="brand">
                    <img class="brand-icon" src="/brand.png" alt="" />
                    <span class="brand-name">"Bunny\u{a0}Hops"</span>
                </div>
                <button class="tab-add"
                    title="Mock-сервер (BunnyProx)"
                    style=move || if mock_mode.get() {
                        "width:auto;padding:0 12px;font-weight:600;font-size:12px;background:var(--accent-dim);color:var(--accent)".to_string()
                    } else {
                        "width:auto;padding:0 12px;font-weight:600;font-size:12px".to_string()
                    }
                    on:click=move |_| mock_mode.update(|m| *m = !*m)>{ "Mock" }</button>
                <button class={move || if invite_save.get() {
                        "tab-add invite".to_string()
                    } else {
                        "tab-add".to_string()
                    }}
                    title="Сохранить текущий запрос как вкладку"
                    on:click=move |_| open_modal.run(())>
                    <svg width="22" height="22" viewBox="0 0 22 22" aria-hidden="true">
                        <path d="M11 4v14M4 11h14" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" fill="none"/>
                    </svg>
                </button>
                <div class="tab-cloud">
                    <Show when={move || !api_calls.get().is_empty()}>
                        <span class="tab-chip tech">
                            <button class="tab-chip-open"
                                on:click={move |_| {
                                    api_panel.update(|v| *v = !*v);
                                    refresh_api.run(());
                                }}>
                                <span class="tab-chip-name">"automation_rules"</span>
                            </button>
                            <span class="api-count">{move || api_calls.get().len()}</span>
                        </span>
                    </Show>
                    <For
                        each={move || saved.get().into_iter().collect::<Vec<_>>()}
                        key={|r| r.id.clone()}
                        children={move |r| {
                            let id = r.id.clone();
                            // живые данные чипа читаем из saved — автосохранение обновляет их на месте
                            let cur = {
                                let id = id.clone();
                                std::sync::Arc::new(move || {
                                    saved
                                        .get()
                                        .into_iter()
                                        .find(|x| x.id == id)
                                        .unwrap_or_else(|| r.clone())
                                })
                            };
                            let cur_cls = std::sync::Arc::clone(&cur);
                            let cur_title = std::sync::Arc::clone(&cur);
                            let cur_name = std::sync::Arc::clone(&cur);
                            let id_cls = id.clone();
                            view! {
                                <span class={move || {
                                    let r = cur_cls();
                                    let mut c = format!("tab-chip m-{}", r.method);
                                    if active_tab.get().as_deref() == Some(id_cls.as_str()) {
                                        c.push_str(" active");
                                    }
                                    if let Some((fid, t)) = flash.get() {
                                        if fid == id_cls {
                                            c.push_str(&format!(" flash{}", t % 2));
                                        }
                                    }
                                    c
                                }} title={move || cur_title().url}>
                                    <button class="tab-chip-open"
                                        on:click={let id = id.clone(); move |_| load_saved_req.run(id.clone())}>
                                        <span class="tab-chip-name">{move || cur_name().name}</span>
                                    </button>
                                    <button class="tab-chip-del" title="Удалить"
                                        on:click={let id = id.clone(); move |_| del_confirm.set(Some(id.clone()))}>"✕"</button>
                                </span>
                            }
                        }}
                    />
                </div>
                
                <div class=move || format!("cmd-search {}", if search_open.get() { "cmd-open" } else if mac { "cmd-mac" } else { "cmd-pc" })>
                    <Show when={move || !search_open.get()}>
                        <button class="cmd-pill" type="button"
                            title="Поиск по экрану" on:click={move |_| search_open.set(true)}>
                            <span class="cmd-label">"Search"</span>
                            <span class="cmd-keys" aria-hidden="true">
                                <kbd class=move || format!("cmd-key{}", if held_mod.get() { " on" } else { "" })>{mod_label}</kbd>
                                <kbd class=move || format!("cmd-key{}", if held_k.get() { " on" } else { "" })>"K"</kbd>
                            </span>
                        </button>
                    </Show>
                    <Show when={move || search_open.get()}>
                        <div class="cmd-field">
                            <svg class="cmd-ico" viewBox="0 0 24 24" fill="none" stroke="currentColor"
                                stroke-width="2" stroke-linecap="round" aria-hidden="true">
                                <circle cx="11" cy="11" r="7"></circle>
                                <path d="m20 20-3.5-3.5"></path>
                            </svg>
                            <input id="hop-screen-search" class="cmd-input" type="text" placeholder="Search"
                                value=move || search_q.get()
                                on:input=move |ev| {
                                    let v = input_value(ev);
                                    search_q.set(v.clone());
                                    search_hits.set(hop_search(&v));
                                }
                                on:keydown=move |ev: web_sys::KeyboardEvent| {
                                    if ev.key() == "Escape" {
                                        close_search();
                                    }
                                }
                                on:blur=move |_| {
                                    if search_q.get_untracked().is_empty() {
                                        close_search();
                                    }
                                }
                            />
                            <span class="cmd-count">{move || {
                                let q = search_q.get();
                                let n = search_hits.get();
                                if q.is_empty() {
                                    String::new()
                                } else if n == 0 {
                                    "нет совпадений".to_string()
                                } else {
                                    format!("совпадений: {n}")
                                }
                            }}</span>
                            <kbd class="cmd-key">"Esc"</kbd>
                        </div>
                    </Show>
                </div>
            </header>
            <Show when={move || !groups.get().is_empty()}>
                <div class="group-bar">
                    {move || {
                        groups
                            .get()
                            .into_iter()
                            .map(|g| {
                                let id = g.id.clone();
                                let name = g.name.clone();
                                view! {
                                    <button class="group-dot" title=name
                                        on:click={let id = id.clone(); move |_| restore_group.run(id.clone())}>
                                        {g.num.to_string()}
                                    </button>
                                }
                            })
                            .collect::<Vec<_>>()
                    }}
                </div>
            </Show>
            <div class="toolbar-row">
                <div class="zoom-controls" title="Масштаб интерфейса">
                    <button class="zoom-btn" disabled=move || scale_idx.get() == SCALES.len() - 1
                        title="Уменьшить шрифт (экономия места)" on:click=move |_| zoom_by(1)>"A−"</button>
                    <span class="zoom-label">{move || format!("{}%", (SCALES[scale_idx.get()] * 100.0).round() as i64)}</span>
                    <button class="zoom-btn" disabled=move || scale_idx.get() == 0
                        title="Увеличить шрифт" on:click=move |_| zoom_by(-1)>"A+"</button>
                </div>
                <div class="urlbar">
                    <MethodSelect
                        get_method=Callback::new(move |_| method.get())
                        set_method=Callback::new(move |v: String| method.set(v.clone()))
                    />
                    <input
                        class="url-input"
                        placeholder=move || if url.get().is_empty() { ph_hint.get() } else { String::new() }
                        spellcheck="false"
                        value=move || url.get()
                        on:input=move |ev| on_url_change.run(input_value(ev))
                        on:keydown=move |ev: web_sys::KeyboardEvent| {
                            if ev.key() == "Enter" && can_send.get() {
                                send.run(());
                            }
                        }
                    />
                    <Show when={move || loading.get() || running.get()} fallback={move || view! {
                        <button class="send-btn" disabled={move || !can_send.get()}
                            on:click=move |_| send.run(())>"Send"</button>
                    }}>
                        <button class="send-btn stop" title="Остановить" on:click=move |_| stop.run(())>"■ Stop"</button>
                    </Show>
                </div>
            </div>

            <Show when={move || url_preview_m.get().is_some()}>
                <div class="vars-preview-line" title="Предпросмотр подстановки переменных">
                    <span class="vars-arrow">"→"</span>
                    {move || {
                        let parts = url_preview_m.get()?;
                        Some(
                            parts
                                .into_iter()
                                .map(|ps| {
                                    let cls = if ps.ok { "vp-ok" } else { "vp-unknown" };
                                    let txt = ps.text;
                                    view! { <span class=cls>{txt}</span> }
                                })
                                .collect::<Vec<_>>(),
                        )
                    }}
                    <Show when={move || loading.get()}><span class="spinner tiny"/></Show>
                </div>
            </Show>

            <Show when={move || error.get().is_some()}>
                <div class="error-banner">"⚠️ "{move || error.get().unwrap_or_default()}</div>
            </Show>

            <Show when={move || !mock_mode.get()}
                  fallback=move || view! { <main class="panes panes-mock"><ProxyView/></main> }>
            <main class="panes">
                <section class="pane">
                    <nav class="tabs">
                        <div class="tab-track" id="req-tabs" node_ref=req_track>
                        <span class="tab-pill"/>
                        <button class=move || if tab.get() == "params" { "tab active" } else { "tab" }
                            on:click=move |_| tab.set("params".to_string())>
                            "URL Params"
                            <Show when={move || enabled_params.get() != 0}>
                                <span class="tab-count">{move || enabled_params.get()}</span>
                            </Show>
                        </button>
                        <button class=move || if tab.get() == "headers" { "tab active" } else { "tab" }
                            on:click=move |_| tab.set("headers".to_string())>
                            "Headers"
                            <Show when={move || enabled_headers.get() != 0}>
                                <span class="tab-count">{move || enabled_headers.get()}</span>
                            </Show>
                        </button>
                        <Show when={move || has_body.get()}>
                            <button class=move || if tab.get() == "body" { "tab active" } else { "tab" }
                                on:click=move |_| tab.set("body".to_string())>
                                "Body"
                                <Show when={move || !body.get().trim().is_empty()}>
                                    <span class="tab-dot"/>
                                </Show>
                            </button>
                        </Show>
                        <button class=move || if tab.get() == "steps" { "tab active" } else { "tab" }
                            on:click=move |_| tab.set("steps".to_string())>
                            "Scenario"
                            <Show when={move || !steps.get().is_empty()}>
                                <span class="tab-count">{move || steps.get().len()}</span>
                            </Show>
                        </button>
                        <button class=move || if tab.get() == "vars" { "tab active" } else { "tab" }
                            on:click=move |_| tab.set("vars".to_string())>
                            "Vars"
                            <Show when={move || !vars_map.get().is_empty()}>
                                <span class="tab-count">{move || vars_map.get().len()}</span>
                            </Show>
                            </button>
                            </div>
                            </nav>

                    <div class="pane-body">
                        <Show when={move || tab.get() == "params"}>
                            <KvEditor
                                get_rows=Callback::new(move |_: ()| params.get())
                                set_rows=Callback::new(move |rs: Vec<KvRow>| on_params_change.run(rs))
                                key_placeholder="Parameter"
                                value_placeholder="Value"
                            />
                        </Show>
                        <Show when={move || tab.get() == "headers"}>
                            <KvEditor
                                get_rows=Callback::new(move |_: ()| headers.get())
                                set_rows=Callback::new(move |rs: Vec<KvRow>| headers.set(rs))
                                key_placeholder="Header"
                                value_placeholder="Value"
                            />
                        </Show>
                        <Show when={move || tab.get() == "body" && has_body.get()}>
                            <div class="body-editor">
                                <div class="copy-row">
                                    <div class="body-hint">{move || {
                                        let b = body.get();
                                        if pretty_json(&b).is_some() {
                                            "✓ JSON valid".to_string()
                                        } else if !b.trim().is_empty() {
                                            "— не JSON".to_string()
                                        } else {
                                            String::new()
                                        }
                                    }}</div>
                                    <Show when={move || !body.get().trim().is_empty()}>
                                        <CopyButton
                                            title="Копировать форматированный JSON запроса"
                                            ok=copied_req.read_only()
                                            on_copied=mark_copied(copied_req)
                                            get_text=Callback::new(move |_: ()| {
                                                let b = body.get_untracked();
                                                pretty_json(&b).unwrap_or(b)
                                            })
                                        />
                                    </Show>
                                </div>
                                <textarea
                                    class="body-textarea"
                                    placeholder="{\"hello\": \"world\"}"
                                    spellcheck="false"
                                    bind:value=body
                                />
                            </div>
                        </Show>
                        <Show when={move || tab.get() == "steps"}>
                            <div class="steps-panel">
                                <div class="steps-actions">
                                    <button class="btn-accent" disabled=move || running.get() || steps.get().is_empty()
                                        title="Выполнить шаги последовательно" on:click=move |_| run_all.run(())>
                                        "▶ Run all"
                                    </button>
                                    <button class="btn-ghost" disabled=move || running.get() || url.get().trim().is_empty()
                                        title="Добавить настроенный сейчас запрос как шаг"
                                        on:click=move |_| add_step_from_current.run(())>
                                        "＋ Текущий запрос"
                                    </button>
                                    <button class="btn-ghost" disabled=move || running.get() || steps.get().is_empty()
                                        title="Сбросить статусы" on:click=move |_| reset_steps.run(())>"↺ Reset"</button>
                                    <button class="btn-ghost danger" disabled=move || running.get() || steps.get().is_empty()
                                        title="Очистить список" on:click=move |_| clear_steps.run(())>"🗑"</button>
                                </div>
                                <Show when={move || steps.get().is_empty()}>
                                    <div class="kv-empty">
                                        "Сценарий пуст. Настройте запрос (метод, URL, заголовки, тело) и нажмите «＋ Текущий запрос»."
                                    </div>
                                </Show>
                                <For
                                    each={move || steps.get().into_iter().enumerate().collect::<Vec<_>>()}
                                    key={|(_, s)| s.id.clone()}
                                    children={move |(i, step)| {
                                    view! {
                                        <div class={move || format!("step-card st-{}", step_at(&steps, i, |s| s.status.clone()))} data-step={step.id.clone()}>
                                            <div class="step-head" on:click=move |_| {
                                                let cur = step_at_opt(&steps, i, |s| s.id.clone());
                                                let o = open_step_id.get_untracked();
                                                open_step_id.set(if o == cur { None } else { cur });
                                            }>
                                                <span class="step-num">{i + 1}</span>
                                                <span class=move || format!("step-method m-{}", step_at(&steps, i, |s| s.method.clone()))>
                                                    {move || step_at(&steps, i, |s| s.method.clone())}
                                                </span>
                                                <span class="step-url" title={move || step_at(&steps, i, |s| s.url.clone())}>
                                                    {move || step_at(&steps, i, |s| s.url.clone())}
                                                </span>
                                                <Show when={move || !step_at(&steps, i, |s| s.capture_name.clone()).is_empty()}>
                                                    <span class="step-capture" title={move || "подставляется как {{".to_string() + &step_at(&steps, i, |s| s.capture_name.clone()) + "}}"}>
                                                        {move || step_at(&steps, i, |s| s.capture_name.clone())}
                                                    </span>
                                                </Show>
                                                <span class="step-status">
                                                    <Show when={move || step_at(&steps, i, |s| s.status.clone()) == "running"}>
                                                        <span class="spinner"/>
                                                        <span class="streaming-badge mini">{format!("● шаг {}", i + 1)}</span>
                                                    </Show>
                                                    <Show when={move || step_at(&steps, i, |s| s.status.clone()) == "ok"}>
                                                        {move || step_at_opt(&steps, i, |s| s.result.clone()).flatten().map(result_chip)}
                                                    </Show>
                                                    <Show when={move || step_at(&steps, i, |s| s.status.clone()) == "fail"}>
                                                        {move || match step_at_opt(&steps, i, |s| s.result.clone()).flatten() {
                                                            Some(r) => fail_chip(r),
                                                            None => view! { <span class="status-chip s-5xx mini">"✗"</span> }.into_any(),
                                                        }}
                                                    </Show>
                                                    <Show when={move || step_at(&steps, i, |s| s.status.clone()) == "skipped"}>
                                                        <span class="chips-skipped">"— пропуск"</span>
                                                    </Show>
                                                </span>
                                            </div>
                                            <Show when={move || open_step_id.get() == step_at_opt(&steps, i, |s| s.id.clone())}>
                                                <div class="step-body">
                                                    <div class="step-row">
                                                        <MethodSelect
                                                            get_method=Callback::new(move |_| step_at(&steps, i, |s| s.method.clone()))
                                                            set_method=Callback::new(move |v: String| {
                                                                update_step_at(&steps, i, |s| s.method = v);
                                                            })
                                                        />
                                                        <input class="kv-input" spellcheck="false" placeholder="URL (можно {{var}})"
                                                            value=move || step_at(&steps, i, |s| s.url.clone())
                                                            on:input=move |ev| {
                                                                let v = input_value(ev);
                                                                update_step_at(&steps, i, |s| s.url = v);
                                                            }/>
                                                    </div>
                                                    <div class="step-headers-label">"Заголовки шага"</div>
                                                    <KvEditor
                                                        get_rows=Callback::new(move |_: ()| steps.get_untracked().get(i).map(|s| s.headers.clone()).unwrap_or_default())
                                                        set_rows=Callback::new(move |rs: Vec<KvRow>| {
                                                            steps.update(|v| { if let Some(s) = v.get_mut(i) { s.headers = rs; } });
                                                        })
                                                        key_placeholder="Header"
                                                        value_placeholder="Value"
                                                    />
                                                    <Show when={move || !is_bodyless(&step_at(&steps, i, |s| s.method.clone()))}>
                                                        <textarea class="body-textarea step-body-textarea"
                                                            placeholder="{\"hello\": \"{{name}}\"}" spellcheck="false"
                                                            on:input=move |ev| {
                                                                let v = input_value(ev);
                                                                update_step_at(&steps, i, |s| s.body = v);
                                                            }>{step_at_opt(&steps, i, |s| s.body.clone()).unwrap_or_default()}</textarea>
                                                    </Show>
                                                    <div class="step-row capture-row">
                                                        <label>"Захват переменной:"</label>
                                                        <input class="kv-input" style="max-width:120px" placeholder="имя"
                                                            spellcheck="false"
                                                            value=move || step_at(&steps, i, |s| s.capture_name.clone())
                                                            on:input=move |ev| {
                                                                let v: String = input_value(ev).chars()
                                                                    .filter(|c| c.is_alphanumeric() || *c == '_')
                                                                    .collect();
                                                                update_step_at(&steps, i, |s| s.capture_name = v);
                                                            }/>
                                                        <input class="kv-input" placeholder="$.data.token"
                                                            spellcheck="false"
                                                            value=move || step_at(&steps, i, |s| s.capture_path.clone())
                                                            on:input=move |ev| {
                                                                let v = input_value(ev);
                                                                update_step_at(&steps, i, |s| s.capture_path = v);
                                                            }/>
                                                        <button class="kv-remove" title="Удалить шаг"
                                                            on:click=move |_| {
                                                                if let Some(sid) = step_at_opt(&steps, i, |s| s.id.clone()) {
                                                                    remove_step.run(sid);
                                                                }
                                                            }>"×"</button>
                                                    </div>
                                                </div>
                                            </Show>
                                        </div>
                                    }
                                    }}
                                />
                                <Show when={move || running.get()}>
                                    <div class="steps-hint">"▶ Выполняем по шагам… правая панель показывает ответ текущего шага."</div>
                                </Show>
                            </div>
                        </Show>
                        <Show when={move || tab.get() == "vars"}>
                            <div>
                                <div class="kv-empty" style="margin-bottom:8px">
                                    "Переменные подставляются в URL, заголовки и тело через "<code>"{{name}}"</code>
                                </div>
                                <KvEditor
                                    get_rows=Callback::new(move |_: ()| vars.get())
                                    set_rows=Callback::new(move |rs: Vec<KvRow>| vars.set(rs))
                                    key_placeholder="baseUrl"
                                    value_placeholder="https://api.example.com"
                                />
                            </div>
                        </Show>
                    </div>
                </section>

                <section class="pane">
                    <Show when={move || response.get().is_some()} fallback={move || view! {
                        <div class="empty-state">
                            <Show when={move || loading.get() || running.get()} fallback={move || view! {
                                <p>"Отправьте запрос, чтобы увидеть ответ"</p>
                                <p class="empty-sub">
                                    "Enter для отправки. Ответ появляется в реальном времени, пока API ещё отвечает. \
                                     Сценарий (Steps) позволяет пройти цепочку запросов пошагово. CORS не мешает — \
                                     запросы идут через локальный прокси."
                                </p>
                            }}>
                                <span class="spinner big"/>
                                <p>{move || if running.get() { "Выполняется сценарий…" } else { "Ожидаем заголовки ответа…" }}</p>
                            </Show>
                        </div>
                    }}>
                        <div class="response-meta">
                            {move || {
                                let r = response.get()?;
                                Some(view! {
                                    <span class=format!("status-chip s-{}xx", r.status / 100)>
                                        {format!("{} {}", r.status, r.status_text)}
                                    </span>
                                })
                            }}
                            <Show when={move || response.get().map(|r| r.streaming).unwrap_or(false)}>
                                <span class="streaming-badge">"● поток"</span>
                            </Show>
                            <Show when={move || response.get().map(|r| r.cancelled).unwrap_or(false)}>
                                <span class="cancelled-badge">"⏹ остановлено"</span>
                            </Show>
                            {move || {
                                let r = response.get()?;
                                Some(view! {
                                    <span class="meta-item">{format!("{:.0} ms", r.time_ms)}</span>
                                    <span class="meta-item">{format_size(Some(r.bytes))}</span>
                                })
                            }}
                            <Show when={move || {
                                let r = response.get();
                                match r.as_ref() {
                                    Some(resp) => !resp.final_url.is_empty() && resp.final_url != url.get(),
                                    None => false,
                                }
                            }}>
                                <span class="meta-item redirect" title="После редиректов">
                                    {move || format!("→ {}", response.get().map(|r| r.final_url).unwrap_or_default())}
                                </span>
                            </Show>
                            <Show when={move || {
                                let r = response.get();
                                match r.as_ref() {
                                    Some(resp) => resp.is_textual && !resp.streaming && !resp.body_text.is_empty(),
                                    None => false,
                                }
                            }}>
                                <CopyButton
                                    title="Копировать форматированный JSON ответа"
                                    ok=copied_resp.read_only()
                                    on_copied=mark_copied(copied_resp)
                                    get_text=Callback::new(move |_: ()| {
                                        if let Some(resp) = response.get_untracked() {
                                            pretty_json(&resp.body_text).unwrap_or(resp.body_text)
                                        } else {
                                            String::new()
                                        }
                                    })
                                />
                            </Show>
                            <Show when={move || {
                                let r = response.get();
                                match r.as_ref() {
                                    Some(resp) => resp.is_textual && !resp.streaming,
                                    None => false,
                                }
                            }}>
                                <div class="body-view-toggle">
                                    <button class=move || if pretty.get() { "view-btn active" } else { "view-btn" }
                                        on:click=move |_| pretty.set(true)>"Pretty"</button>
                                    <button class=move || if !pretty.get() { "view-btn active" } else { "view-btn" }
                                        on:click=move |_| pretty.set(false)>"Raw"</button>
                                </div>
                            </Show>
                        </div>
                        <nav class="tabs">
                            <div class="tab-track" id="resp-tabs" node_ref=resp_track>
                            <span class="tab-pill"/>
                            <button class=move || if resp_tab.get() == "body" { "tab active" } else { "tab" }
                                on:click=move |_| resp_tab.set("body".to_string())>"Response body"</button>
                            <button class=move || if resp_tab.get() == "headers" { "tab active" } else { "tab" }
                                on:click=move |_| resp_tab.set("headers".to_string())>"Response headers"</button>
                            </div>
                        </nav>
                        <Show when={move || {
                            let r = response.get();
                            r.as_ref().map(|resp| resp.is_textual).unwrap_or(false) && resp_tab.get() == "body"
                        }}>
                            <div class="search-bar">
                                <span class="search-icon">"🔍"</span>
                                <input id="hop-search-input" class="search-input" spellcheck="false"
                                    placeholder="Search"
                                    value=move || query.get()
                                    on:input=move |ev| query.set(input_value(ev))
                                    on:keydown=move |ev: web_sys::KeyboardEvent| {
                                        if ev.key() == "Enter" {
                                            ev.prevent_default();
                                            goto_match(if ev.shift_key() { -1 } else { 1 });
                                        }
                                    }/>
                                <span class=move || if match_count.get() != 0 { "search-count" } else { "search-count none" }>
                                    {move || {
                                        let q = query.get();
                                        let mc = match_count.get();
                                        if q.is_empty() {
                                            String::new()
                                        } else if mc == 0 {
                                            "нет совпадений".to_string()
                                        } else {
                                            format!("{} / {}", cur_match.get() + 1, mc)
                                        }
                                    }}
                                </span>
                                <button class="search-nav" disabled=move || match_count.get() == 0
                                    title="Предыдущее (Shift+Enter)" on:click=move |_| goto_match(-1)>"↑"</button>
                                <button class="search-nav" disabled=move || match_count.get() == 0
                                    title="Следующее (Enter)" on:click=move |_| goto_match(1)>"↓"</button>
                                <button class="search-nav" title="Очистить" on:click=move |_| query.set(String::new())>"✕"</button>
                            </div>
                        </Show>
                        <div class="pane-body">
                            <Show when={move || resp_tab.get() == "body"}>
                                {move || {
                                    let r = response.get()?;
                                    if let Some(src) = r.image_blob_url.as_ref() {
                                        return Some(view! {
                                            <img class="response-image" src={src.clone()} alt="response"/>
                                        }.into_any());
                                    }
                                    if r.is_textual {
                                        let lines = search.get().map(|s| s.lines.clone()).unwrap_or_default();
                                        return Some(view! {
                                            <div class="response-lines">
                                                <For
                                                    each={move || lines.clone().into_iter().enumerate().collect::<Vec<_>>()}
                                                    key=|(li, _)| *li
                                                    children=move |(_, line)| {
                                                        view! {
                                                            <div class={move || {
                                                                let mut cls = String::from("code-line");
                                                                if line.hit { cls.push_str(" line-match"); }
                                                                if line.has_error { cls.push_str(" line-error"); }
                                                                if line.segs.iter().any(|s| s.mi == Some(cur_match.get())) {
                                                                    cls.push_str(" line-current");
                                                                }
                                                                cls
                                                            }}>
                                                                {line.segs.clone().into_iter().enumerate().map(|(_si, seg)| match seg.mi {
                                                                    Some(mi) => view! {
                                                                        <mark data-mi={mi}
                                                                            class={move || if cur_match.get() == mi { "match current" } else { "match" }}>
                                                                            {seg.text.clone()}
                                                                        </mark>
                                                                    }.into_any(),
                                                                    None if seg.err => view! {
                                                                        <span class="err-phrase">{seg.text.clone()}</span>
                                                                    }.into_any(),
                                                                    None => view! { <span>{seg.text.clone()}</span> }.into_any(),
                                                                }).collect::<Vec<_>>()}
                                                            </div>
                                                        }
                                                    }
                                                />
                                            </div>
                                        }.into_any());
                                    }
                                    Some(view! {
                                        <div class="binary-note">{move || {
                                            let r = response.get()?;
                                            Some(if r.streaming {
                                                format!("⏬ Получаем бинарные данные… {}", format_size(Some(r.bytes)))
                                            } else {
                                                let ct = if r.content_type.is_empty() { "unknown".to_string() } else { r.content_type.clone() };
                                                format!("Двоичные данные ({}), {}", ct, format_size(Some(r.bytes)))
                                            })
                                        }}</div>
                                    }.into_any())
                                }}
                            </Show>
                            <Show when={move || resp_tab.get() == "headers"}>
                                <table class="headers-table">
                                    <tbody>
                                        {move || {
                                            let r = response.get()?;
                                            Some(r.headers.iter().map(|(k, v)| view! {
                                                <tr>
                                                    <td class="header-name">{k.clone()}</td>
                                                    <td class="header-value">{v.clone()}</td>
                                                </tr>
                                            }).collect::<Vec<_>>())
                                        }}
                                    </tbody>
                                </table>
                            </Show>
                        </div>
                    </Show>
                </section>
            </main>
            </Show>
            <Show when={move || api_panel.get()}>
                <div class="modal-overlay"
                    on:click=move |ev| {
                        let on_overlay = ev
                            .target()
                            .and_then(|t| t.dyn_ref::<web_sys::HtmlElement>().cloned())
                            .map(|t| t.class_list().contains("modal-overlay"))
                            .unwrap_or(false);
                        if on_overlay {
                            api_panel.set(false);
                        }
                    }>
                    <div class="modal api-modal">
                        <div class="api-panel-head">
                            <span>"запросы, присланные в API"</span>
                            <span class="api-head-btns">
                                <button class="api-refresh" title="Обновить"
                                    on:click={move |_| refresh_api.run(())}>"↻"</button>
                                <button class="api-refresh" title="Закрыть (Esc)"
                                    on:click={move |_| api_panel.set(false)}>"✕"</button>
                            </span>
                        </div>
                        <div class="api-list">
{move || {
                            api_calls
                                .get()
                                .into_iter()
                                .rev()
                                .map(|c| {
                                    let c2 = c.clone();
                                    let mcls = format!("api-m m-{}", c.method);
                                    let m = c.method.clone();
                                    let u = c.url.clone();
                                    let st = c.status.to_string();
                                    let t = fmt_ts(c.ts);
                                    view! {
                                        <div class="api-item"
                                            on:click={move |_| load_api_call.run(c2.clone())}>
                                            <span class=mcls>{m}</span>
                                            <span class="api-u">{u}</span>
                                            <span class="api-s">{st}</span>
                                            <span class="api-t">{t}</span>
                                        </div>
                                    }
                                })
                                .collect::<Vec<_>>()
                        }}
                        </div>
                    </div>
                </div>
            </Show>
            <Show when={move || modal_open.get()}>
                <div class="modal-overlay"
                    on:click=move |ev| {
                        let on_overlay = ev
                            .target()
                            .and_then(|t| t.dyn_ref::<web_sys::HtmlElement>().cloned())
                            .map(|t| t.class_list().contains("modal-overlay"))
                            .unwrap_or(false);
                        if on_overlay {
                            modal_open.set(false);
                        }
                    }>
                    <div class="modal">
                        <div class="modal-title">"Новая вкладка"</div>
                        <input class="modal-input" type="text" placeholder="Название вкладки"
                            autofocus=true
                            value=move || modal_name.get()
                            on:input=move |ev| {
                                modal_name.set(input_value(ev));
                                modal_err.set(String::new());
                            }
                            on:keydown=move |ev| {
                                if ev.key() == "Enter" {
                                    save_current.run(());
                                } else if ev.key() == "Escape" {
                                    modal_open.set(false);
                                }
                            } />
                        <Show when={move || !modal_err.get().is_empty()}>
                            <div class="modal-err">{move || modal_err.get()}</div>
                        </Show>
                        <div class="modal-actions">
                            <button class="modal-save" on:click=move |_| save_current.run(())>"Сохранить"</button>
                            <button class="modal-cancel" on:click=move |_| modal_open.set(false)>"Отмена"</button>
                        </div>
                    </div>
                </div>
            </Show>
            <Show when={move || del_confirm.get().is_some()}>
                <div class="modal-overlay"
                    on:click=move |ev| {
                        let on_overlay = ev
                            .target()
                            .and_then(|t| t.dyn_ref::<web_sys::HtmlElement>().cloned())
                            .map(|t| t.class_list().contains("modal-overlay"))
                            .unwrap_or(false);
                        if on_overlay {
                            del_confirm.set(None);
                        }
                    }>
                    <div class="modal">
                        <div class="modal-title">"Удалить вкладку?"</div>
                        <div class="modal-actions">
                            <button class="modal-save danger" on:click=move |_| {
                                if let Some(id) = del_confirm.get_untracked() {
                                    delete_saved.run(id);
                                }
                                del_confirm.set(None);
                            }>"Удалить"</button>
                            <button class="modal-cancel" on:click=move |_| del_confirm.set(None)>"Отмена"</button>
                        </div>
                    </div>
                </div>
            </Show>
            <Show when={move || group_modal.get()}>
                <div class="modal-overlay"
                    on:click=move |ev| {
                        let on_overlay = ev
                            .target()
                            .and_then(|t| t.dyn_ref::<web_sys::HtmlElement>().cloned())
                            .map(|t| t.class_list().contains("modal-overlay"))
                            .unwrap_or(false);
                        if on_overlay {
                            group_modal.set(false);
                        }
                    }>
                    <div class="modal">
                        <div class="modal-title">"Облако заполнено"</div>
                        <div class="modal-hint">"Все вкладки объединятся в группу. Клик по значку группы вернёт их в облако."</div>
                        <input class="modal-input" type="text" placeholder="Имя группы"
                            autofocus=true
                            value=move || group_name.get()
                            on:input=move |ev| group_name.set(input_value(ev))
                            on:keydown=move |ev| {
                                if ev.key() == "Enter" {
                                    save_group.run(());
                                } else if ev.key() == "Escape" {
                                    group_modal.set(false);
                                }
                            } />
                        <div class="modal-actions">
                            <button class="modal-save" on:click=move |_| save_group.run(())>"Создать группу"</button>
                            <button class="modal-cancel" on:click=move |_| group_modal.set(false)>"Отмена"</button>
                        </div>
                    </div>
                </div>
            </Show>
            <button class="actlog-toggle"
                on:click=move |_| {
                    if !log_open.get_untracked() {
                        log_h.set(60);
                    }
                    log_open.update(|o| *o = !*o);
                }>
                "Logs"
                <span class="actlog-count">{move || act_log.get().len()}</span>
            </button>
            <Show when=move || log_open.get()>
                <div class="actlog" style=move || {
                    let cap = tab_bottom_gap().unwrap_or(i32::MAX);
                    format!("height:{}px", log_h.get().min(cap))
                }>
                    <div class="actlog-resize"
                        title="Потянуть вверх/вниз — высота журнала"
                        on:pointerdown=move |ev: web_sys::PointerEvent| {
                            crate::ui::start_log_drag(&ev, log_h);
                        }></div>
                    <div class="actlog-body">
                        {move || {
                            let v: Vec<String> = act_log.get().into_iter().rev().take(100).collect();
                            v.into_iter()
                                .map(|line| view! { <div class="actlog-line">{line}</div> })
                                .collect::<Vec<_>>()
                        }}
                    </div>
                </div>
            </Show>
        </div>
    }
}

/* помещаются ли все чипы облака в одну строку хедера */
