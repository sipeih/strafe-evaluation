// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use inputbot::KeybdKey::*;
use inputbot::MouseButton::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread::sleep;
use std::time::{Duration, SystemTime};
use tauri::AppHandle;
use tauri::Manager;
use winapi::um::winuser::GetKeyboardLayout;

const SHOT_WINDOW_MS: u128 = 300;

#[derive(Clone, serde::Serialize)]
struct Payload {
    strafe_type: String,
    duration: u128,
    shot_delay: Option<u128>,
}

#[derive(Clone)]
struct PendingStrafe {
    payload: Payload,
    timestamp: SystemTime,
}

struct GameState {
    gun_fire_mode: AtomicBool,
    weapon_active: AtomicBool,
    pending_strafe: Mutex<Option<PendingStrafe>>,
}

#[tauri::command]
fn set_gun_fire_mode(state: tauri::State<Arc<GameState>>, enabled: bool) {
    state.gun_fire_mode.store(enabled, Ordering::Relaxed);
}

// Helper to handle emission logic
fn handle_strafe_emission(
    app: &AppHandle,
    state: &Arc<GameState>,
    payload: Payload,
) {
    let gun_mode = state.gun_fire_mode.load(Ordering::Relaxed);
    let weapon_active = state.weapon_active.load(Ordering::Relaxed);

    if !gun_mode {
        // Normal mode: emit immediately
        app.emit_all("strafe", payload).unwrap();
    } else {
        // Gun fire mode
        if weapon_active {
            // Store as pending
            let mut pending = state.pending_strafe.lock().unwrap();
            *pending = Some(PendingStrafe {
                payload,
                timestamp: SystemTime::now(),
            });
        }
        // If weapon not active, ignore strafe
    }
}

fn eval_understrafe(
    elapsed: Duration,
    released_time: &mut Option<SystemTime>,
    app: AppHandle,
    state: Arc<GameState>,
) {
    let time_passed = elapsed.as_micros();
    let mut payload = None;

    if time_passed < (200 * 1000) && time_passed > (1600) {
        payload = Some(Payload {
            strafe_type: "Early".into(),
            duration: time_passed,
            shot_delay: None,
        });
    } else if time_passed < 1600 {
        payload = Some(Payload {
            strafe_type: "Perfect".into(),
            duration: 0,
            shot_delay: None,
        });
    }

    if let Some(p) = payload {
        handle_strafe_emission(&app, &state, p);
    }
    *released_time = None;
}

fn eval_overstrafe(
    elapsed: Duration,
    both_pressed_time: &mut Option<SystemTime>,
    app: AppHandle,
    state: Arc<GameState>,
) {
    let time_passed = elapsed.as_micros();
    if time_passed < (200 * 1000) {
        let payload = Payload {
            strafe_type: "Late".into(),
            duration: time_passed,
            shot_delay: None,
        };
        handle_strafe_emission(&app, &state, payload);
    }
    *both_pressed_time = None;
}

fn is_azerty_layout() -> bool {
    unsafe {
        let layout = GetKeyboardLayout(0);
        let layout_id = layout as u32 & 0xFFFF;
        return matches!(layout_id, 0x040C | 0x080C | 0x140C | 0x180C);
    }
}

fn main() {
    let game_state = Arc::new(GameState {
        gun_fire_mode: AtomicBool::new(true),
        weapon_active: AtomicBool::new(true),
        pending_strafe: Mutex::new(None),
    });

    tauri::Builder::default()
        .manage(game_state.clone())
        .invoke_handler(tauri::generate_handler![set_gun_fire_mode])
        .setup(move |app| {
            let handle = app.handle();
            let state = game_state.clone();

            tauri::async_runtime::spawn(async move {
                let mut left_pressed = false;
                let mut right_pressed = false;
                let mut both_pressed_time: Option<SystemTime> = None;
                let mut right_released_time: Option<SystemTime> = None;
                let mut left_released_time: Option<SystemTime> = None;
                let is_azerty = is_azerty_layout();

                // Mouse state tracking for click detection (simple edge detection)
                let mut last_left_click = false;

                loop {
                    // Tickrate
                    sleep(Duration::from_millis(1));

                    // 1. Weapon/Utility Key Detection
                    if Numrow1Key.is_pressed() || Numrow2Key.is_pressed() || QKey.is_pressed() {
                        state.weapon_active.store(true, Ordering::Relaxed);
                    }
                    if Numrow3Key.is_pressed()
                        || Numrow4Key.is_pressed()
                        || Numrow5Key.is_pressed()
                        || ZKey.is_pressed()
                        || XKey.is_pressed()
                        || CKey.is_pressed()
                        || VKey.is_pressed()
                    {
                        state.weapon_active.store(false, Ordering::Relaxed);
                    }

                    // 2. Gun Fire Detection (Left Click)
                    let current_left_click = LeftButton.is_pressed();
                    if current_left_click && !last_left_click {
                        // Mouse Pressed
                        if state.gun_fire_mode.load(Ordering::Relaxed) {
                            let mut pending_lock = state.pending_strafe.lock().unwrap();
                            if let Some(pending) = &*pending_lock {
                                match pending.timestamp.elapsed() {
                                    Ok(elapsed) => {
                                        let elapsed_ms = elapsed.as_millis();
                                        if elapsed_ms < SHOT_WINDOW_MS {
                                            // Valid shot!
                                            let mut final_payload = pending.payload.clone();
                                            final_payload.shot_delay = Some(elapsed_ms);
                                            handle
                                                .emit_all("strafe", final_payload)
                                                .unwrap();
                                            
                                            // Clear pending
                                            *pending_lock = None;
                                        }
                                    }
                                    Err(_) => {}
                                }
                            }
                        }
                    }
                    last_left_click = current_left_click;

                    // 3. Clean up old pending strafes
                    if state.gun_fire_mode.load(Ordering::Relaxed) {
                         let mut pending_lock = state.pending_strafe.lock().unwrap();
                         let mut should_clear = false;
                         if let Some(pending) = &*pending_lock {
                             if let Ok(elapsed) = pending.timestamp.elapsed() {
                                 if elapsed.as_millis() >= SHOT_WINDOW_MS {
                                     should_clear = true;
                                 }
                             }
                         }
                         if should_clear {
                             *pending_lock = None;
                         }
                    } else {
                        // Ensure pending is clear if mode is off
                        let mut pending_lock = state.pending_strafe.lock().unwrap();
                         if pending_lock.is_some() {
                             *pending_lock = None;
                         }
                    }


                    // 4. Movement Key Detection
                    if right_pressed && !DKey.is_pressed() && !RightKey.is_pressed() {
                        // D released
                        right_pressed = false;
                        let _ = handle.emit_all("d-released", ());
                        right_released_time = Some(SystemTime::now());
                    }
                    if left_pressed
                        && (is_azerty || !AKey.is_pressed())
                        && (!is_azerty || !QKey.is_pressed())
                        && !LeftKey.is_pressed()
                    {
                        // A released
                        left_pressed = false;
                        let _ = handle.emit_all("a-released", ());
                        left_released_time = Some(SystemTime::now());
                    }

                    if ((!is_azerty && AKey.is_pressed())
                        || (is_azerty && QKey.is_pressed())
                        || LeftKey.is_pressed())
                        && !left_pressed
                    {
                        // A pressed
                        left_pressed = true;
                        let _ = handle.emit_all("a-pressed", ());
                        match right_released_time {
                            None => {}
                            Some(x) => match x.elapsed() {
                                Ok(elapsed) => eval_understrafe(
                                    elapsed,
                                    &mut right_released_time,
                                    handle.clone(),
                                    state.clone(),
                                ),
                                Err(e) => {
                                    println!("Error: {e:?}");
                                }
                            },
                        }
                    }

                    if (DKey.is_pressed() || RightKey.is_pressed()) && !right_pressed {
                        // D pressed
                        right_pressed = true;
                        let _ = handle.emit_all("d-pressed", ());
                        match left_released_time {
                            None => {}
                            Some(x) => match x.elapsed() {
                                Ok(elapsed) => eval_understrafe(
                                    elapsed,
                                    &mut left_released_time,
                                    handle.clone(),
                                    state.clone(),
                                ),
                                Err(e) => {
                                    println!("Error: {e:?}");
                                }
                            },
                        }
                    }

                    // Evaluation
                    if left_pressed && right_pressed && both_pressed_time == None {
                        both_pressed_time = Some(SystemTime::now());
                    }

                    if (!left_pressed || !right_pressed) && both_pressed_time != None {
                        match both_pressed_time {
                            None => {}
                            Some(x) => {
                                match x.elapsed() {
                                    Ok(elapsed) => {
                                        // Overlap time
                                        eval_overstrafe(
                                            elapsed,
                                            &mut both_pressed_time,
                                            handle.clone(),
                                            state.clone(),
                                        )
                                    }
                                    Err(e) => {
                                        println!("Error: {e:?}");
                                    }
                                }
                            }
                        }
                    }
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to run app");
}
