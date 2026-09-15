# Copyright (c) 2026 Glavo
# SPDX-License-Identifier: MPL-2.0

janex() {
    case "${1-}" in
        use|deactivate)
            local _janex_arg _janex_code
            for _janex_arg in "$@"; do
                case "$_janex_arg" in
                    --project|--help|-h|--shell|--shell=*)
                        @JANEX_EXE@ "$@"
                        return $?
                        ;;
                esac
            done
            _janex_code=$(@JANEX_EXE@ "$@" --shell sh) || return $?
            eval "$_janex_code"
            hash -r 2>/dev/null || :
            ;;
        *) @JANEX_EXE@ "$@" ;;
    esac
}
