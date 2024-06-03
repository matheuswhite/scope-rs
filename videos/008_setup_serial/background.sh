#!/bin/bash

socat -dd PTY,link=COM4,raw,echo=0 PTY,link=COM4_out &> /dev/null

