#!/usr/bin/env bash
# Waybar custom module — Orka server status
# Config example:
#   "custom/orka": {
#     "exec": "~/.local/bin/orka-status.sh",
#     "return-type": "json",
#     "interval": 10
#   }

if systemctl is-active --quiet orka-server.service; then
	echo '{"text": "orka", "tooltip": "Orka server is running", "class": "active"}'
elif systemctl is-enabled --quiet orka-server.service 2>/dev/null; then
	echo '{"text": "orka", "tooltip": "Orka server is stopped", "class": "warning"}'
else
	echo '{"text": "orka", "tooltip": "Orka server is not installed", "class": "inactive"}'
fi
