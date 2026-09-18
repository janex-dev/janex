# Copyright (c) 2026 Glavo
# SPDX-License-Identifier: MPL-2.0

set -eu
original_java=$JAVA_HOME
janex_home=$JANEX_HOME
unset JANEX_HOME
. "$janex_home/shell/init.sh"
test "$JANEX_HOME" = "$janex_home"
test "${JANEX_SHELL_STATE+x}" != x
test "${GRADLE_HOME+x}" != x
test "$JAVA_HOME" = "$original_java"
original_path=$PATH
. "$JANEX_HOME/shell/init.sh"
test "$PATH" = "$original_path"
janex activate
test "$GRADLE_HOME" = "$JANEX_TEST_FIRST"
test "$(gradle)" = fixture
janex use sdk:gradle@8.14.3
test "$GRADLE_HOME" = "$JANEX_TEST_SECOND"
janex activate
test "$GRADLE_HOME" = "$JANEX_TEST_SECOND"
case ":$PATH:" in *":$JANEX_TEST_FIRST/bin:"*) exit 20;; esac
previous_path=$PATH
previous_state=$JANEX_SHELL_STATE
if janex use sdk:gradle@8.14.2 sdk:maven@99.0.0 2>/dev/null; then exit 21; fi
test "$PATH" = "$previous_path"
test "$JANEX_SHELL_STATE" = "$previous_state"
result=0
janex use --invalid-option 2>/dev/null || result=$?
test "$result" = 2
janex use
test "$GRADLE_HOME" = "$JANEX_TEST_FIRST"
cd "$JANEX_TEST_OTHER"
janex use
test "$GRADLE_HOME" = "$JANEX_TEST_SECOND"
janex use --project sdk:gradle@8.14.2
test "$GRADLE_HOME" = "$JANEX_TEST_SECOND"
janex use
test "$GRADLE_HOME" = "$JANEX_TEST_FIRST"
janex activate
janex deactivate
test "$PATH" = "$original_path"
test "$JAVA_HOME" = "$original_java"
test "${GRADLE_HOME+x}" != x
test "${JANEX_SHELL_STATE+x}" != x
test "$(command -v janex)" = janex
janex --version
janex activate
test "$GRADLE_HOME" = "$JANEX_TEST_FIRST"
janex deactivate
test "$PATH" = "$original_path"
