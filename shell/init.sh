# Copyright (c) 2026 Glavo
# SPDX-License-Identifier: MPL-2.0

_janex_init() {
    local _janex_code
    _janex_code=$("${JANEX_HOME-$HOME/.janex}/bin/janex" init sh) || return $?
    eval "$_janex_code"
}
if _janex_init; then
    unset -f _janex_init
else
    unset -f _janex_init
    return 1
fi
