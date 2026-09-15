# Copyright (c) 2026 Glavo
# SPDX-License-Identifier: MPL-2.0

function _janex_init
    set -l code (command @JANEX_EXE@ init --shell fish | string collect)
    set -l result $pipestatus[1]
    if test $result -ne 0
        return $result
    end
    if not set -q JANEX_HOME
        set -gx JANEX_HOME @JANEX_HOME@
    end
    eval "$code"
end
if _janex_init
    functions -e _janex_init
else
    functions -e _janex_init
    return 1
end
