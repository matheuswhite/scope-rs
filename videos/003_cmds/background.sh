#!/bin/bash

socat -dd PTY,link=COM1,raw,echo=0 PTY,link=COM1_out &> /dev/null

