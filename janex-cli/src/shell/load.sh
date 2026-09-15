# Copyright (c) 2026 Glavo
# SPDX-License-Identifier: MPL-2.0

_janex_init() {
    local _janex_code
    _janex_code=$(@JANEX_EXE@ init --shell sh) || return $?
    if [ "${JANEX_HOME+x}" != x ]; then
        export JANEX_HOME=@JANEX_HOME@
    fi
    eval "$_janex_code"
}
if _janex_init; then
    unset -f _janex_init
else
    unset -f _janex_init
    return 1
fi
