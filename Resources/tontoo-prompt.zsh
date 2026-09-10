# TontooOS Terminal prompt: green user@machine folder %, nothing else.
# Sourced last by the generated $ZDOTDIR/.zshrc, so it wins over themes
# while aliases, completions and the rest of the user config keep working.
# %# renders % for users and # for root.
PROMPT='%F{2}%n@%m %~ %#%f '

# Report the current folder to VTE (OSC 7) so the title bar and the
# title-click-to-Finder always see the real CWD. Stock zsh never sends
# this on its own, without it VTE keeps an empty directory uri.
_tontoo_osc7_cwd() {
  local host="${HOST:-$(hostname)}"
  local dir="${PWD:gs/%/%25}"
  dir="${dir:gs/ /%20}"
  printf '\e]7;file://%s%s\a' "$host" "$dir"
}
autoload -Uz add-zsh-hook
add-zsh-hook precmd _tontoo_osc7_cwd
add-zsh-hook chpwd _tontoo_osc7_cwd
