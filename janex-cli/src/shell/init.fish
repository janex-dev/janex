# Copyright (c) 2026 Glavo
# SPDX-License-Identifier: MPL-2.0

if not contains -- @JANEX_BIN@ $PATH
    set -gx PATH @JANEX_BIN@ $PATH
end

if not set -q JANEX_HOME
    set -gx JANEX_HOME @JANEX_HOME@
end
if not contains -- "$JANEX_HOME/bin" $PATH
    set -gx PATH "$JANEX_HOME/bin" $PATH
end

function janex
    if test (count $argv) -gt 0; and contains -- $argv[1] activate use deactivate
        for arg in $argv
            if contains -- $arg --project --help -h --shell; or string match -q -- '--shell=*' $arg
                command @JANEX_EXE@ $argv
                return $status
            end
        end
        set -l code (command @JANEX_EXE@ $argv --shell fish | string collect)
        set -l result $pipestatus[1]
        if test $result -ne 0
            return $result
        end
        eval "$code"
    else
        command @JANEX_EXE@ $argv
    end
end
true
