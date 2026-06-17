-- Maki_home.spine.lua
--
-- 完全镜像 Blue Archive Live2D 壁纸 (maki_home/wpengine/page.js) 的动画逻辑。
-- 原代码是一个 Live2D 模型，这里用 Spine 实现相同的状态机。
--
-- ===== API 参考 =====
--   play(track, name, looping)              -- 设置轨道动画
--   add(track, name, looping, delay)        -- 排队动画
--   clear_track(track)                      -- 立即清空轨道
--   empty(track, mix_duration)              -- 淡出轨道（对应 setEmptyAnimation）
--   animations() -> {name,...}              -- 所有动画名
--   has_animation(name) -> bool             -- 检查动画存在
--   random_from({...}) -> any               -- 从表中随机选一项
--   math.*, string.*, table.*              -- Lua 安全标准库子集
--
-- ===== 引擎回调 =====
--   on_init(anim_table)                     -- 骨架加载完毕
--   on_update(dt)                           -- 每帧 dt 秒
--   on_complete(track, animation_name)      -- 动画完成（非循环）
--
-- ===== 配置 =====

skeleton = "Maki_home.json"
scale = 0.0
offset_x = 0.0
offset_y = 0.0

-- ======================================================================
-- 状态机 — 对应 Live2D 的 isIdle / state / listeners
-- ======================================================================
-- 三个状态:
--   "start"    — 播完 Start_Idle_01 就切到 idle
--   "idle"     — Idle_01 循环播放，is_idle = true，ready to talk
--   "talking"  — Talk 动画在 tracks 1/2 上，播完回到 idle
-- ======================================================================

is_idle = false    -- 对应 Live2D 的 this.isIdle
state = "init"

-- ===== on_init — 对应 Live2D 的 start() + idle() =====

function on_init()
    if has_animation("Start_Idle_01") then
        -- start(): 存在 Start_Idle_01 → 播它，然后等 on_complete 切 idle
        state = "start"
        play(0, "Start_Idle_01", false)
    else
        -- idle(): 没有开场 → 直接 idle 循环
        state = "idle"
        is_idle = true
        play(0, "Idle_01", true)
    end
end

-- ===== on_complete — 对应 startListener + talkListener =====

function on_complete(track, name)
    -- ── startListener.onComplete ────────────────────────────────────
    -- 原代码: onStartComplete(e) → play("Idle_01"), loop=true, isIdle=true
    if state == "start" and name == "Start_Idle_01" then
        state = "idle"
        is_idle = true
        play(0, "Idle_01", true)
        return
    end

    -- ── talkListener.onComplete ─────────────────────────────────────
    -- 原代码: onTalkComplete(e):
    --   check: 1 === e.trackIndex || 2 === e.trackIndex
    --     → isIdle = true
    --     → setEmptyAnimation(1, 0.2)    -- 淡出 talk body 轨道
    --     → setEmptyAnimation(2, 0.2)    -- 淡出 arm 轨道
    --     → removeListener(talkListener)
    if state == "talking" and (track == 1 or track == 2) then
        -- 淡出 talk 轨道，露出底下一直在播的 Idle_01 (track 0)
        empty(1, 0.2)
        empty(2, 0.2)
        is_idle = true
        state = "idle"
        -- 注意：track 0 的 Idle_01 从未停止，淡出 talk 轨道后自然可见
    end
end

-- ===== randomTalk — 对应 Live2D 的 randomTalk() =====
--
-- 原代码逻辑:
--   1. 守卫: char 存在 && isIdle
--   2. getAnimations().filter(e => startsWith("Talk_") && endsWith("_M"))
--   3. 随机选一个
--   4. 如果以 _M 结尾 → 替换 _A 播在 track 2
--     否则 → clearTrack(2)
--   5. isIdle = false
--   6. setAnimation(1, name, false)
--   7. addListener(talkListener)   ← 在本实现中由 state == "talking" 的 on_complete 替代

function trigger_talk()
    -- 守卫: 必须 idle 才能说话（对应 !this.isIdle return）
    if not is_idle then
        return
    end

    -- 收集 Talk_*_M 动画
    -- 对应: this.getAnimations().filter(e => e.startsWith("Talk_") && e.endsWith("_M"))
    local pool = {}
    for _, name in ipairs(animations()) do
        if name:match("^Talk_.*_M$") then
            table.insert(pool, name)
        end
    end
    if #pool == 0 then
        return
    end

    -- 随机选一个（对应 Math.floor(Math.random() * e.length)）
    local talk_name = pool[math.random(#pool)]

    -- 如果选中以 _M 结尾 → 对应 _A 版本播在 track 2
    -- 对应原代码: a.endsWith("_M") 分支
    if talk_name:match("_M$") then
        -- "Talk_01_M" → 去掉末尾2字符 ("_M") + "_A" = "Talk_01_A"
        -- 对应 JS: a.slice(0, -2) + "_A"
        -- Lua 注意: :sub(1, -N) 去掉 N-1 个字符, 要去掉2个字符需要用 -3
        local arm_name = talk_name:sub(1, -3) .. "_A"
        if has_animation(arm_name) then
            -- 对应: this.char.state.hasAnimation(e) && this.char.state.setAnimation(2, e, false)
            play(2, arm_name, false)
        end
    else
        -- 对应: else { this.char.state.clearTrack(2); }
        clear_track(2)
    end

    -- 设状态并播放在轨道 1
    -- 对应: this.isIdle = false; this.char.state.setAnimation(1, a, false);
    is_idle = false
    state = "talking"
    play(1, talk_name, false)

    -- 原代码后续: this.char.state.addListener(this.talkListener)
    -- 在本实现中由 state == "talking" 的 on_complete 驱动，无需显式加 listener
end

-- ===== on_update — 自动触发 talk 演示 =====
--
-- 原网页通过 onClick + tapToTalk 配置触发。
-- 这里用随机定时器模拟"空闲一会儿就说话"的效果。
-- 如果将来 on_click 实现，可以直接调用 trigger_talk()。

local talk_timer = 0

function on_update(dt)
    if is_idle then
        talk_timer = talk_timer - dt
        if talk_timer <= 0 then
            -- 每 10~15 分钟随机触发一次
            talk_timer = 600.0 + math.random() * 300.0
            trigger_talk()
        end
    end
end
