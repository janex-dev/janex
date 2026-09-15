# Janex

Place `janex` (`janex.exe` on Windows) in a directory on PATH, or invoke it by its full path.
The optional `janex-launcher` executable is a prefix for building self-launching packages.

Run `janex init` to write shell initialization scripts into `JANEX_HOME/shell`.
`JANEX_HOME` defaults to `~/.janex`; set it to an absolute path to use another directory.
Janex prints the appropriate loading commands and does not edit shell startup files.

For example, in PowerShell:

```powershell
janex init
. "$HOME/.janex/shell/init.ps1"
janex activate
```

For Bash or Zsh, load `shell/init.sh`; for Fish, load `shell/init.fish`.
After moving the executable, run `janex init` again and reload the script.
SDKs and caches remain in `JANEX_HOME`, independently of the executable's location.

The shell scripts are embedded in the executable; they are not separate distribution files.
See the accompanying `LICENSE` for Janex's license terms.
