# Copyright (c) 2026 Glavo
# SPDX-License-Identifier: MPL-2.0

case ":${PATH-}:" in
    *":"@JANEX_BIN@":"*) ;;
    *) export PATH=@JANEX_BIN@${PATH+:"$PATH"} ;;
esac

if [ "${JANEX_HOME+x}" != x ]; then export JANEX_HOME=@JANEX_HOME@; fi
case ":${PATH-}:" in
    *":$JANEX_HOME/bin:"*) ;;
    *) export PATH="$JANEX_HOME/bin"${PATH+:"$PATH"} ;;
esac

janex() {
    case "${1-}" in
        activate|use|deactivate)
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
