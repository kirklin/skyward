#!/sbin/bash
# No block devices, mounts, user files or installer commands are touched.
set -euo pipefail
[[ ${BASH_VERSINFO[0]} == 5 && ${BASH_VERSINFO[1]} == 3 ]]
[[ ${BASH_VERSINFO[2]} == 20 ]]
[[ $# == 3 && $1 == 3 && $2 == 3 && $3 == synthetic.zip ]]
values=(alpha beta)
local_value() { local result=${values[1]}; printf '%s' "$result"; }
[[ $(local_value) == beta ]]
read -r line < <(printf '%s\n' pipe-ok)
[[ $line == pipe-ok ]]
printf 'ui_print bash-smoke-ok\nui_print\n' > "/proc/self/fd/$2"
printf 'bash-smoke-ok: %s\n' "$BASH_VERSION"
