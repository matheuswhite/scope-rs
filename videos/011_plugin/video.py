import keyboard
from time import sleep
import sys

sys.path.insert(0, "..")
from videos.lib import write_input

write_input("!plugin load plugins/echo.lua")
write_input("!echo hello")
write_input("!echo world")
write_input("Hello")
write_input("AT")

sleep(1)
keyboard.press_and_release("esc")
