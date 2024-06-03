#!/bin/bash

echo "start sleeping..."
sleep 6
echo "create serial"
socat -dd PTY,link=COM1,raw,echo=0 PTY,link=COM1_out &> /dev/null &
sleep 10
echo "kill serial"
kill $(echo $!)
sleep 6
echo "create serial again"
socat -dd PTY,link=COM1,raw,echo=0 PTY,link=COM1_out &> /dev/null

