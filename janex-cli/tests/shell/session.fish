# Copyright (c) 2026 Glavo
# SPDX-License-Identifier: MPL-2.0

set -l original_java $JAVA_HOME
set -l janex_home $JANEX_HOME
set -e JANEX_HOME
source "$janex_home/shell/init.fish"; or exit 40
test "$JANEX_HOME" = "$janex_home"; or exit 55
set -q JANEX_SHELL_STATE; and exit 41
set -q GRADLE_HOME; and exit 42
test "$JAVA_HOME" = "$original_java"; or exit 43
set -l original_path (string join : -- $PATH)
source "$JANEX_HOME/shell/init.fish"; or exit 44
test (string join : -- $PATH) = "$original_path"; or exit 45
janex activate; or exit 46
test "$GRADLE_HOME" = "$JANEX_TEST_FIRST"; or exit 10
test (gradle) = fixture; or exit 11
janex use sdk:gradle@8.14.3; or exit 12
test "$GRADLE_HOME" = "$JANEX_TEST_SECOND"; or exit 13
janex activate; or exit 53
test "$GRADLE_HOME" = "$JANEX_TEST_SECOND"; or exit 54
contains -- "$JANEX_TEST_FIRST/bin" $PATH; and exit 14
set -l previous_path (string join : -- $PATH)
set -l previous_state $JANEX_SHELL_STATE
janex use sdk:gradle@8.14.2 sdk:maven@99.0.0 2>/dev/null; and exit 15
test (string join : -- $PATH) = "$previous_path"; or exit 16
test "$JANEX_SHELL_STATE" = "$previous_state"; or exit 17
janex use --invalid-option 2>/dev/null
test $status -eq 2; or exit 32
janex use; or exit 18
test "$GRADLE_HOME" = "$JANEX_TEST_FIRST"; or exit 19
cd "$JANEX_TEST_OTHER"
janex use; or exit 20
test "$GRADLE_HOME" = "$JANEX_TEST_SECOND"; or exit 21
janex use --project sdk:gradle@8.14.2; or exit 22
test "$GRADLE_HOME" = "$JANEX_TEST_SECOND"; or exit 23
janex use; or exit 24
test "$GRADLE_HOME" = "$JANEX_TEST_FIRST"; or exit 25
janex activate; or exit 47
janex deactivate; or exit 26
test (string join : -- $PATH) = "$original_path"; or exit 27
test "$JAVA_HOME" = "$original_java"; or exit 28
set -q GRADLE_HOME; and exit 29
set -q JANEX_SHELL_STATE; and exit 30
functions -q janex; or exit 31
janex --version; or exit 48
janex activate; or exit 49
test "$GRADLE_HOME" = "$JANEX_TEST_FIRST"; or exit 50
janex deactivate; or exit 51
test (string join : -- $PATH) = "$original_path"; or exit 52
exit 0
