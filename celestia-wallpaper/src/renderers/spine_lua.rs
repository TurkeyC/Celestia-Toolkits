//! Lua animation scripting engine for Spine skeletons.
//!
//! Loads a `.spine.lua` script and exposes a Rust → Lua API for
//! controlling Spine animation playback.  The script can define
//! callback functions that the engine calls at the right moments:
//!
//! - `on_init(anim_table)`  – after skeleton is loaded, before first frame
//! - `on_update(dt)`        – every frame (dt in seconds)
//! - `on_complete(track, name)` – when a non-looping animation finishes
//!
//! Lua globals provided by the engine:
//! - `play(track, name, looping)`    – set animation on a track
//! - `add(track, name, looping, delay)` – queue onto a track
//! - `clear_track(track)`            – remove animation from a track
//! - `animations()`                  – table of all animation names
//! - `has_animation(name)`           – bool check
//!

#![cfg(feature = "lua-scripting")]

use std::error::Error;
use std::sync::{Arc, Mutex};

use rusty_spine::{EventType, SkeletonController};

// ======================================================================
// Command queue — enqueued by Lua, drained and applied by the engine.
// ======================================================================

#[derive(Debug)]
pub(crate) enum Command {
    Play {
        track: i32,
        name: String,
        looping: bool,
    },
    Add {
        track: i32,
        name: String,
        looping: bool,
        delay: f32,
    },
    ClearTrack(i32),
    /// Transition to no animation with a crossfade.
    Empty {
        track: i32,
        mix_duration: f32,
    },
}

// ======================================================================
// Shared state between rusty_spine listener and Lua closures.
// ======================================================================

struct LuaShared {
    /// Completed animations reported by rusty_spine's listener.
    completed: Mutex<Vec<(i32, String)>>,
    /// Commands queued by Lua `play()`/`add()`/`clear_track()` calls.
    commands: Mutex<Vec<Command>>,
    /// Cached animation list.
    anim_list: Vec<String>,
}

// ======================================================================
// LuaRuntime
// ======================================================================

pub(crate) struct LuaRuntime {
    lua: mlua::Lua,
    shared: Arc<LuaShared>,

    pub(crate) has_on_update: bool,
    pub(crate) has_on_complete: bool,
    pub(crate) has_on_init: bool,
}

impl LuaRuntime {
    /// Create a new Lua runtime, load the script, register Rust API
    /// functions, and attach the rusty_spine event listener.
    ///
    /// `controller` is used to read the animation list and to attach
    /// the listener.  The controller is NOT kept after this call —
    /// all animation changes go through the command queue.
    pub(crate) fn new(
        script: &str,
        controller: &mut SkeletonController,
    ) -> Result<Self, Box<dyn Error>> {
        // Collect animation list from the skeleton data.
        let anim_list: Vec<String> = controller
            .skeleton
            .data()
            .animations()
            .map(|a| a.name().to_string())
            .collect();

        // ---- build shared state ------------------------------------------
        let shared = Arc::new(LuaShared {
            completed: Mutex::new(Vec::new()),
            commands: Mutex::new(Vec::new()),
            anim_list,
        });

        // ---- create Lua VM -----------------------------------------------
        // Load only safe standard libraries — no io, os, debug, package,
        // ffi, or jit that could be abused.
        let safe_stdlibs = mlua::StdLib::MATH
            | mlua::StdLib::STRING
            | mlua::StdLib::TABLE;
        let lua = mlua::Lua::new_with(safe_stdlibs, mlua::LuaOptions::new())
            .map_err(|e| format!("Failed to create Lua VM: {e}"))?;
        let globals = lua.globals();

        // ---- register spine API functions --------------------------------

        // play(track, name, looping)
        {
            let shared = shared.clone();
            let func = lua.create_function(move |_, (track, name, looping): (i32, String, bool)| {
                if let Ok(mut cmds) = shared.commands.lock() {
                    cmds.push(Command::Play { track, name, looping });
                }
                Ok(())
            })?;
            globals.set("play", func)?;
        }

        // add(track, name, looping, delay)
        {
            let shared = shared.clone();
            let func = lua.create_function(
                move |_, (track, name, looping, delay): (i32, String, bool, f32)| {
                    if let Ok(mut cmds) = shared.commands.lock() {
                        cmds.push(Command::Add {
                            track,
                            name,
                            looping,
                            delay,
                        });
                    }
                    Ok(())
                },
            )?;
            globals.set("add", func)?;
        }

        // clear_track(track)
        {
            let shared = shared.clone();
            let func = lua.create_function(move |_, track: i32| {
                if let Ok(mut cmds) = shared.commands.lock() {
                    cmds.push(Command::ClearTrack(track));
                }
                Ok(())
            })?;
            globals.set("clear_track", func)?;
        }

        // empty(track, mix_duration) — fade to no animation
        {
            let shared = shared.clone();
            let func = lua.create_function(move |_, (track, mix): (i32, f32)| {
                if let Ok(mut cmds) = shared.commands.lock() {
                    cmds.push(Command::Empty { track, mix_duration: mix });
                }
                Ok(())
            })?;
            globals.set("empty", func)?;
        }

        // animations() -> table
        {
            let anim_names = shared.anim_list.clone();
            let func = lua.create_function(move |lua: &mlua::Lua, ()| {
                let t = lua.create_table()?;
                for (i, name) in anim_names.iter().enumerate() {
                    t.raw_set(i as i64 + 1, name.clone())?;
                }
                Ok(t)
            })?;
            globals.set("animations", func)?;
        }

        // has_animation(name) -> bool
        {
            let shared = shared.clone();
            let func = lua.create_function(move |_, name: String| {
                Ok(shared.anim_list.iter().any(|a| a == &name))
            })?;
            globals.set("has_animation", func)?;
        }

        // ---- inject a Lua helper: random_from(tbl) -----------------------
        // Implemented in Lua so it naturally uses math.random.
        lua.load(
            r#"
            function random_from(tbl)
                local n = #tbl
                if n == 0 then return nil end
                return tbl[math.random(n)]
            end
            "#,
        )
        .exec()
        .map_err(|e| format!("Failed to inject random_from: {e}"))?;

        // ---- attach rusty_spine listener ---------------------------------
        {
            let shared = shared.clone();
            controller.animation_state.set_listener(move |_state, event_type, entry, _event| {
                if let EventType::Complete = event_type {
                    if let Ok(mut q) = shared.completed.lock() {
                        q.push((
                            entry.track_index(),
                            entry.animation().name().to_string(),
                        ));
                    }
                }
            });
        }

        // ---- execute the user script -------------------------------------
        lua.load(script)
            .exec()
            .map_err(|e| format!("Lua script error: {e}"))?;

        // ---- detect which callbacks are defined --------------------------
        let has_on_update = Self::has_function(&lua, "on_update");
        let has_on_complete = Self::has_function(&lua, "on_complete");
        let has_on_init = Self::has_function(&lua, "on_init");

        Ok(Self {
            lua,
            shared,
            has_on_update,
            has_on_complete,
            has_on_init,
        })
    }

    // ---- helpers ---------------------------------------------------------

    fn has_function(lua: &mlua::Lua, name: &str) -> bool {
        lua.globals()
            .raw_get::<mlua::Value>(name)
            .ok()
            .map_or(false, |v| matches!(v, mlua::Value::Function(_)))
    }

    fn call_opt(&self, name: &str, args: mlua::MultiValue) {
        let Ok(func) = self.lua.globals().raw_get::<mlua::Function>(name) else { return };
        if let Err(e) = func.call::<()>(args) {
            log::error!("Lua callback '{}' error: {}", name, e);
        }
    }

    // ---- public API called by spine.rs -----------------------------------

    /// Call `on_init(animations_table)`.
    pub(crate) fn call_init(&self) {
        if !self.has_on_init {
            return;
        }
        // Build the animations table for the callback.
        let tbl = self
            .lua
            .create_table()
            .expect("Lua table creation should not fail");
        for (i, name) in self.shared.anim_list.iter().enumerate() {
            let _ = tbl.raw_set(i as i64 + 1, name.clone());
        }
        self.call_opt("on_init", mlua::MultiValue::from_vec(vec![mlua::Value::Table(tbl)]));
    }

    /// Call `on_update(dt)`.
    pub(crate) fn call_update(&self, dt: f32) {
        if !self.has_on_update {
            return;
        }
        self.call_opt("on_update", mlua::MultiValue::from_vec(vec![mlua::Value::Number(dt as f64)]));
    }

    /// Call `on_complete(track, animation_name)` for each completed
    /// animation reported by the spine listener.
    pub(crate) fn call_completions(&self) {
        if !self.has_on_complete {
            // Still need to drain the queue even if no callback.
            if let Ok(mut q) = self.shared.completed.lock() {
                q.clear();
            }
            return;
        }
        let items: Vec<(i32, String)> = if let Ok(mut q) = self.shared.completed.lock() {
            std::mem::take(&mut *q)
        } else {
            return;
        };
        for (track, name) in items {
            self.call_opt(
                "on_complete",
                mlua::MultiValue::from_vec(vec![
                    mlua::Value::Integer(track as i64),
                    mlua::Value::String(self.lua.create_string(&name).expect("valid string")),
                ]),
            );
        }
    }

    /// Drain the command queue and return it for the engine to apply.
    pub(crate) fn drain_commands(&self) -> Vec<Command> {
        if let Ok(mut cmds) = self.shared.commands.lock() {
            std::mem::take(&mut *cmds)
        } else {
            Vec::new()
        }
    }
}
