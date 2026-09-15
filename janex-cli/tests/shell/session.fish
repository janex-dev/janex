# Copyright (c) 2026 Glavo
# SPDX-License-Identifier: MPL-2.0

set -l original_path (string join : -- $PATH)
set -l original_java $JAVA_HOME
$JANEX_TEST_EXE activate fish | source
test "$GRADLE_HOME" = "$JANEX_TEST_FIRST"; or exit 10
test (gradle) = fixture; or exit 11
janex use gradle@8.14.3; or exit 12
test "$GRADLE_HOME" = "$JANEX_TEST_SECOND"; or exit 13
contains -- "$JANEX_TEST_FIRST/bin" $PATH; and exit 14
set -l previous_path (string join : -- $PATH)
set -l previous_state $JANEX_SHELL_STATE
janex use gradle@8.14.2 maven@99.0.0 2>/dev/null; and exit 15
test (string join : -- $PATH) = "$previous_path"; or exit 16
test "$JANEX_SHELL_STATE" = "$previous_state"; or exit 17
janex use --invalid-option 2>/dev/null
test $status -eq 2; or exit 32
janex use; or exit 18
test "$GRADLE_HOME" = "$JANEX_TEST_FIRST"; or exit 19
cd "$JANEX_TEST_OTHER"
janex use; or exit 20
test "$GRADLE_HOME" = "$JANEX_TEST_SECOND"; or exit 21
janex use --project gradle@8.14.2; or exit 22
test "$GRADLE_HOME" = "$JANEX_TEST_SECOND"; or exit 23
janex use; or exit 24
test "$GRADLE_HOME" = "$JANEX_TEST_FIRST"; or exit 25
$JANEX_TEST_EXE activate fish | source
janex deactivate; or exit 26
test (string join : -- $PATH) = "$original_path"; or exit 27
test "$JAVA_HOME" = "$original_java"; or exit 28
set -q GRADLE_HOME; and exit 29
set -q JANEX_SHELL_STATE; and exit 30
functions -q janex; and exit 31
exit 0
