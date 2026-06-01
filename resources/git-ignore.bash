# Bash completion for git-ignore
_git_ignore_completions() {
    local cur prev templates
    COMPREPLY=()
    cur="${COMP_WORDS[COMP_CWORD]}"
    prev="${COMP_WORDS[COMP_CWORD-1]}"
    
    local opts="-u --update -p --patch -l --list -i --info -v --version --compact"

    # Do not complete if the current word is a known flag
    if [[ "$cur" == -* ]]; then
        COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
        return 0
    fi

    # Dynamically call git-ignore --list to get available templates
    templates=$(git-ignore --list 2>/dev/null)
    if [[ -z "$templates" ]]; then
        return 0
    fi

    if [[ "$cur" == *,* ]]; then
        # Handle comma-separated lists (e.g., Node,Ru<TAB>)
        local prefix="${cur%,*}"
        local suffix="${cur##*,}"
        
        # Find matches for the part after the last comma
        local matches=$(compgen -W "${templates}" -- "${suffix}")
        
        for m in ${matches}; do
            COMPREPLY+=("${prefix},${m}")
        done
    else
        # Simple completion for the first template
        COMPREPLY=( $(compgen -W "${templates}" -- "${cur}") )
    fi
}

complete -o nospace -F _git_ignore_completions git-ignore
