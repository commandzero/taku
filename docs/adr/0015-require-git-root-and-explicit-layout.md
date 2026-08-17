# Require a Git worktree root and explicit layout

A Taku Project must be initialized exactly at Git's reported worktree root, including worktrees where `.git` is a file, and Taku may inspect but never mutate Git history or staging state. Initialization explicitly selects Single or Multi layout and records Environment names in `.taku/project.yml`; an interactive invocation may prompt for missing choices, while non-interactive use must provide them, and `.taku/applications/<application>/*.yml` remains tracked while local Context and Observed State files are ignored.
