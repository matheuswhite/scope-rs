# Send in Hexadecimal: `$` starts a hex sequence; spaces/commas/dashes separate.
send_line '$48-65-6c-6c-6f'
send_line '$48,65,6c,6c,6f'
send_line '$48,65-6c,6c6f0a'
sleep 1
press Escape
