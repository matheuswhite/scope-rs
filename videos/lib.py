import keyboard
import random
from time import sleep
from enum import Enum
from serial import Serial
from threading import Thread
from subprocess import Popen, PIPE


class Color(Enum):
    Red = b"\x1b[31m"
    Green = b"\x1b[32m"
    Yellow = b"\x1b[33m"
    Blue = b"\x1b[34m"
    Magenta = b"\x1b[35m"
    Cyan = b"\x1b[36m"
    Gray = b"\x1b[37m"


def to_color(message: bytes, color: Color) -> bytes:
    return color.value + message + b"\x1b[0m"


def ansi_color_task(time_to_die: int = 0) -> Thread:

    def task():
        color_pool = [
            Color.Red,
            Color.Green,
            Color.Yellow,
            Color.Blue,
            Color.Magenta,
            Color.Cyan,
            Color.Gray,
        ]

        with Serial("COM1_out") as s:
            for _ in range(time_to_die * 2):
                sleep(0.5)
                message = b""
                for _ in range(3):
                    color = random.choice(color_pool)
                    message += to_color(b"Hello, World!", color) + b" "
                message += b"\r\n"
                s.write(message)

    return Thread(target=task, daemon=True)


def invisibles_task(time_to_die: int = 0) -> Thread:

    def task():
        with Serial("COM1_out") as s:
            for _ in range(time_to_die * 2):
                sleep(0.5)
                message = b"Hello, "
                message += bytes(map(lambda x: x + 0x7E, b"World"))
                message += b" \0Again\r\n"
                s.write(message)

    return Thread(target=task, daemon=True)


def write_input(msg, type_speed=0.15, wait_msg=0.25, wait_end=0.5):
    for c in msg:
        keyboard.write(c)
        sleep(type_speed)
    sleep(wait_msg)
    keyboard.press_and_release("enter")
    sleep(wait_end)


def repeat_key(key, times, type_speed=0.15, wait_msg=0.25, wait_end=0.5):
    for _ in range(times):
        keyboard.press_and_release(key)
        sleep(type_speed)
    sleep(wait_msg)
    keyboard.press_and_release("enter")
    sleep(wait_end)


def kill_socat():
    Popen(["pkill", "socat"], stdout=PIPE, stderr=PIPE)


def spawn_socat():
    Popen(
        ["socat", "PTY,link=COM1,raw,echo=0", "PTY,link=COM1_out,raw"],
        stdout=PIPE,
        stderr=PIPE,
    )
