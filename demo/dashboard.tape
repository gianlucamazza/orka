Output demo/orka-dashboard.gif
Set Theme "Catppuccin Mocha"
Set Width 960
Set Height 600
Set FontSize 14
Set TypingSpeed 50ms

# Requires: orka-server running with observe.backend = "otel"
Type "orka dashboard --interval 3"
Enter
Sleep 4s

# Let it auto-refresh once, then manually refresh
Type "r"
Sleep 4s

Type "r"
Sleep 3s

Type "q"
Sleep 1s
