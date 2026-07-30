# Agent Notes

## Canonical installed runtime

amux is normally installed as a TPM plugin. The canonical production checkout
and hook launcher path is `~/.tmux/plugins/amux/bin/amux`, not the source
checkout currently being inspected.

Do not report hooks that invoke `~/.tmux/plugins/amux/bin/amux` as install or
configuration drift merely because they differ from the current repository
path. Treat that path as expected. Only report hook drift when their rendered
event mapping, matcher, command shape, or plugin version is genuinely stale or
incorrect relative to the TPM checkout being used.
