#!/usr/bin/env python3
"""Pseudo-terminal that answers like a Quectel RM520N-EU AT port, for installer UI tests.

Prints the terminal path, then serves until killed. ADB starts disabled (usbcfg field 0) and
switches on when the installer writes the configuration back with the field set to 2.
"""
import os, pty, re, sys, tty

master, slave = pty.openpty()
tty.setraw(slave)
print(os.ttyname(slave), flush=True)
usb = ["0x2C7C", "0x0801", "1", "1", "1", "1", "1", "0", "0"]
fixed = {
    "AT": "",
    "AT+CGMI": "Quectel",
    "AT+GMM": "RM520N-EU",
    "AT+GMR": "RM520NEUAAR03A01M4G",
    "AT+CGSN": "861234567890123",
    'AT+QMAP="MPDN_RULE"': '+QMAP: "MPDN_rule",0,0,0,0,0\r\n+QMAP: "MPDN_rule",1,0,0,0,0',
    "AT+QTEMP": '+QTEMP: "soc-thermal","43"',
    "AT+CSQ": "+CSQ: 24,99",
}
buffer = b""
while True:
    try:
        data = os.read(master, 1024)
    except OSError:
        break
    buffer += data
    while b"\r" in buffer:
        raw, buffer = buffer.split(b"\r", 1)
        command = raw.decode(errors="replace").strip()
        if not command:
            continue
        body, ok = "", True
        if command.upper() == 'AT+QCFG="USBCFG"':
            body = '+QCFG: "usbcfg",' + ",".join(usb)
        elif command.upper().startswith('AT+QCFG="USBCFG",'):
            usb = command.split(",", 1)[1].split(",")
        elif command.upper() == "AT+QSIMDET?":
            ok = False
        elif command in fixed:
            body = fixed[command]
        elif re.match(r"AT\+[A-Z]+(\?|=)?", command, re.I):
            body = "+" + command[3:].split("=")[0].rstrip("?") + ": 1"
        reply = command + "\r\r\n" + (body + "\r\n\r\n" if body else "") + ("OK" if ok else "ERROR") + "\r\n"
        os.write(master, reply.encode())
