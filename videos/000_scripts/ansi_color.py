import random

from serial import Serial
from time import sleep
from enum import Enum


class Color(Enum):
    Red = b'\x1b[31m'
    Green = b'\x1b[32m'
    Yellow = b'\x1b[33m'
    Blue = b'\x1b[34m'
    Magenta = b'\x1b[35m'
    Cyan = b'\x1b[36m'
    Gray = b'\x1b[37m'


def to_color(message: bytes, color: Color) -> bytes:
    return color.value + message + b'\x1b[0m'


if __name__ == '__main__':
    color_pool = [Color.Red, Color.Green, Color.Yellow, Color.Blue, Color.Magenta, Color.Cyan, Color.Gray]

    with Serial('COM1_out') as s:
        while True:
            sleep(0.5)
            message = b''
            for _ in range(3):
                color = random.choice(color_pool)
                message += to_color(b'Hello, World!', color) + b' '
            message += b'\r\n'
            s.write(message)
