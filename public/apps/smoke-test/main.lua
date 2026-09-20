-- Smoke-test app: exercises a meaningful chunk of the badge.* API surface
-- (badge.ui widgets + styling + align, badge.input, badge.led, badge.store,
-- badge.sys) to prove the emulator's plumbing works end-to-end.

local title
local counter_label
local bar
local checkbox
local press_count = 0

function on_enter(root)
  title = badge.ui.label(root, "Hack the Badge Emulator")
  title:set_pos(8, 8)
  title:set_size(304, 20)
  title:style({ text_color = 0xffffff, text_align = "center" })

  local btn = badge.ui.button(root, 120, 32)
  btn:set_pos(8, 40)
  btn:style({ bg_color = 0x2255aa, radius = 6, border_color = 0x88aaff, border_width = 1 })
  local btn_label = badge.ui.label(btn, "Press A")
  btn_label:align("center", 0, 0)
  btn_label:style({ text_color = 0xffffff })

  bar = badge.ui.bar(root, 0, 100, 25)
  bar:set_pos(8, 88)
  bar:set_size(304, 16)
  bar:style({ bg_color = 0x222222, radius = 4 })
  bar:style({ bg_color = 0x33cc66 }, "indicator")

  checkbox = badge.ui.checkbox(root, "LED on press", false)
  checkbox:set_pos(8, 116)

  counter_label = badge.ui.label(root, "ms: 0")
  counter_label:set_pos(8, 200)
  counter_label:set_size(304, 20)
  counter_label:style({ text_color = 0x88ccff })

  local saved = badge.store.get_int("presses")
  if saved then
    press_count = saved
    title:set_text("Welcome back! presses=" .. tostring(press_count))
  end

  badge.sys.log("smoke_test: on_enter complete")
end

function on_button(button, kind)
  if kind ~= badge.input.KIND.PRESSED then
    return
  end

  if button == badge.input.BUTTON.A then
    press_count = press_count + 1
    title:set_text("Button A pressed x" .. tostring(press_count))
    bar:set_value((press_count * 15) % 100)
    checkbox:set_checked(true)

    badge.led.set_all(0, 255, 80)
    badge.led.show()

    badge.store.set_int("presses", press_count)
    badge.sys.log("A pressed, press_count=" .. tostring(press_count))
  elseif button == badge.input.BUTTON.B then
    checkbox:set_checked(false)
    badge.led.clear()
    badge.led.show()
    badge.sys.log("B pressed: reset LEDs")
  end
end

function on_tick()
  if counter_label then
    counter_label:set_text("ms: " .. tostring(badge.sys.ms()))
  end
end

function on_exit()
  badge.sys.log("smoke_test: on_exit")
end
