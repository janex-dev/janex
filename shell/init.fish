# Copyright (c) 2026 Glavo
# SPDX-License-Identifier: MPL-2.0

function _janex_init
    set -l janex_home "$HOME/.janex"
    if set -q JANEX_HOME
        set janex_home "$JANEX_HOME"
    end
    set -l code (command "$janex_home/bin/janex" init fish | string collect)
    set -l result $pipestatus[1]
    if test $result -ne 0
        return $result
    end
    eval "$code"
end
if _janex_init
    functions -e _janex_init
else
    functions -e _janex_init
    return 1
end
