// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use inputbot::KeybdKey::*;
use inputbot::MouseButton::*;
use std::sync::{
    Arc, Mutex,
};
use std::thread::{self, sleep};
use std::time::{Duration, SystemTime};
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
    pending_strafe: Mutex<Option<PendingStrafe>>,
}

// Helper to handle emission logic
fn handle_strafe_emission(
    state: &Arc<GameState>,
    payload: Payload,
) {
    // Gun fire mode (ALWAYS ON)
    // Store as pending
    if let Ok(mut pending) = state.pending_strafe.lock() {
        *pending = Some(PendingStrafe {
            payload,
            timestamp: SystemTime::now(),
        });
    } else {
        eprintln!("Failed to lock pending_strafe mutex");
    }
}

fn eval_understrafe(
    elapsed: Duration,
    released_time: &mut Option<SystemTime>,
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
        handle_strafe_emission(&state, p);
    }
    *released_time = None;
}

fn eval_overstrafe(
    elapsed: Duration,
    both_pressed_time: &mut Option<SystemTime>,
    state: Arc<GameState>,
) {
    let time_passed = elapsed.as_micros();
    if time_passed < (300 * 1000) {
        let payload = Payload {
            strafe_type: "Late".into(),
            duration: time_passed,
            shot_delay: None,
        };
        handle_strafe_emission(&state, payload);
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
        pending_strafe: Mutex::new(None),
    });

    tauri::Builder::default()
        .manage(game_state.clone())
        .setup(move |app| {
            let handle = app.handle();
            let state = game_state.clone();

            // Spawn on a dedicated OS thread instead of Tauri's async runtime
            // This prevents blocking the async runtime with the infinite loop
            thread::spawn(move || {
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

                    // 1. Gun Fire Detection (Left Click)
                    let current_left_click = LeftButton.is_pressed();
                    if current_left_click && !last_left_click {
                        // Mouse Pressed
                        if let Ok(mut pending_lock) = state.pending_strafe.lock() {
                            if let Some(pending) = &*pending_lock {
                                match pending.timestamp.elapsed() {
                                    Ok(elapsed) => {
                                        let elapsed_ms = elapsed.as_millis();
                                        if elapsed_ms < SHOT_WINDOW_MS {
                                            // Valid shot!
                                            let mut final_payload = pending.payload.clone();
                                            final_payload.shot_delay = Some(elapsed_ms);
                                            
                                            if let Err(e) = handle.emit_all("strafe", final_payload) {
                                                eprintln!("Failed to emit strafe: {}", e);
                                            }
                                            
                                            // Clear pending
                                            *pending_lock = None;
                                        }
                                    }
                                    Err(_) => {}
                                }
                            }
                        } else {
                            eprintln!("Failed to lock pending_strafe mutex (mouse click)");
                        }
                    }
                    last_left_click = current_left_click;

                    // 2. Clean up old pending strafes
                    // Use a separate scope or check to avoid holding lock too long
                    let mut should_clear = false;
                    if let Ok(pending_lock) = state.pending_strafe.lock() {
                        if let Some(pending) = &*pending_lock {
                            if let Ok(elapsed) = pending.timestamp.elapsed() {
                                if elapsed.as_millis() >= SHOT_WINDOW_MS {
                                    should_clear = true;
                                }
                            }
                        }
                    }
                    
                    if should_clear {
                        if let Ok(mut pending_lock) = state.pending_strafe.lock() {
                            *pending_lock = None;
                        }
                    }

                    // 3. Movement Key Detection
                    if right_pressed && !DKey.is_pressed() && !RightKey.is_pressed() {
                        // D released
                        right_pressed = false;
                        if let Err(e) = handle.emit_all("d-released", ()) {
                             eprintln!("Failed to emit d-released: {}", e);
                        }
                        right_released_time = Some(SystemTime::now());
                    }
                    if left_pressed
                        && (is_azerty || !AKey.is_pressed())
                        && (!is_azerty || !QKey.is_pressed())
                        && !LeftKey.is_pressed()
                    {
                        // A released
                        left_pressed = false;
                        if let Err(e) = handle.emit_all("a-released", ()) {
                            eprintln!("Failed to emit a-released: {}", e);
                        }
                        left_released_time = Some(SystemTime::now());
                    }

                    if ((!is_azerty && AKey.is_pressed())
                        || (is_azerty && QKey.is_pressed())
                        || LeftKey.is_pressed())
                        && !left_pressed
                    {
                        // A pressed
                        left_pressed = true;
                        if let Err(e) = handle.emit_all("a-pressed", ()) {
                            eprintln!("Failed to emit a-pressed: {}", e);
                        }
                        match right_released_time {
                            None => {}
                            Some(x) => match x.elapsed() {
                                Ok(elapsed) => eval_understrafe(
                                    elapsed,
                                    &mut right_released_time,
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
                        if let Err(e) = handle.emit_all("d-pressed", ()) {
                            eprintln!("Failed to emit d-pressed: {}", e);
                        }
                        match left_released_time {
                            None => {}
                            Some(x) => match x.elapsed() {
                                Ok(elapsed) => eval_understrafe(
                                    elapsed,
                                    &mut left_released_time,
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
