# Fish completion for git-ignore

# Disable file completions
complete -c git-ignore -f

# Add options
complete -c git-ignore -s u -l update -d 'Update the local templates repository'
complete -c git-ignore -s p -l patch -d 'Patch the current .gitignore file instead of overwriting'
complete -c git-ignore -s l -l list -d 'List all available templates'
complete -c git-ignore -s i -l info -d 'Show information about current setup and integrity'
complete -c git-ignore -s v -l version -d 'Show version information'
complete -c git-ignore -l compact -d 'Compact the local database to save space'

# Dynamic template completion
complete -c git-ignore -a "(git-ignore --list 2>/dev/null)"
