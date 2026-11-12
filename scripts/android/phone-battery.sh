#!/bin/bash
# Battery cost of Zeron's agent phone link on a real phone (docs/design/android-phone-tools.md).
#
#   scripts/android/phone-battery.sh start [serial]   # unplugged, screen off, then leave it
#   scripts/android/phone-battery.sh stop  [serial]   # hours later: prints the drain and the app's share
#
# Reads Android's own accounting: the battery level drop per hour, Zeron's estimated power use,
# its radio-active time and wakeups. Run the same hours for each case in the doc and compare.
set -euo pipefail
ADB=(adb)
[[ -n "${2:-}" ]] && ADB=(adb -s "$2")
PKG=sh.zeron.android
STATE=/tmp/zeron-phone-battery.state

case "${1:-}" in
  start)
    plugged=$("${ADB[@]}" shell dumpsys battery | awk '/AC powered|USB powered|Wireless powered/ && /true/ {print}' | wc -l)
    [[ "$plugged" -eq 0 ]] || { echo "unplug the phone first (it is charging)"; exit 1; }
    "${ADB[@]}" shell dumpsys batterystats --reset >/dev/null
    level=$("${ADB[@]}" shell dumpsys battery | awk '/level:/ {print $2}')
    echo "$(date +%s) $level" > "$STATE"
    echo "started at ${level}%. Turn the screen off, disconnect wireless adb if you can, and leave it for hours."
    ;;
  stop)
    read -r t0 l0 < "$STATE"
    l1=$("${ADB[@]}" shell dumpsys battery | awk '/level:/ {print $2}')
    hours=$(awk -v a="$t0" -v b="$(date +%s)" 'BEGIN {printf "%.2f", (b-a)/3600}')
    echo "level $l0% -> $l1% in ${hours} h = $(awk -v a="$l0" -v b="$l1" -v h="$hours" 'BEGIN {printf "%.2f", (a-b)/h}') %/h"
    uid=$("${ADB[@]}" shell pm list packages -U "$PKG" | sed 's/.*uid:\([0-9]*\).*/\1/')
    echo "--- Zeron (uid $uid)"
    "${ADB[@]}" shell dumpsys batterystats --charged "$PKG" | grep -E "Estimated power use|Uid u0a|Mobile radio active|Wifi Running|Wifi Scan|Wake lock|Foreground|Total cpu time|Wifi network|Mobile network" | head -20
    echo "--- the phone"
    "${ADB[@]}" shell dumpsys batterystats --charged | grep -E "Screen off|Time on battery|Deep idle|Mobile radio active time|Wifi on" | head -10
    echo "--- is Zeron exempt from battery optimization?"
    "${ADB[@]}" shell dumpsys deviceidle whitelist | grep "$PKG" || echo "not exempt"
    ;;
  *)
    echo "usage: $0 start|stop [adb serial]"; exit 2 ;;
esac
