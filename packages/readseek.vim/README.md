# readseek.vim

`readseek.vim` is a Vim9script source navigation plugin for
[`readseek`](https://github.com/jarkkojs/readseek). It shares ReadSeek's release
version and installs or upgrades to the matching prebuilt binary when the plugin
loads, unless a `g:readseek_executable` override is configured.
Release binaries are available for macOS ARM64, Linux ARM64/x64, and Windows x64.

Before using the plugin, initialize the readseek map cache, either from a shell:

```sh
readseek init
```

or from Vim with `:ReadSeekInit`, which initializes the cache for the current
project root.

NOTE: this is still highly experimental plugin and somewhat unfinished.

## Installation

### vim-plug

```vim
Plug 'jarkkojs/readseek', {'rtp': 'packages/readseek.vim'}
```

### Manual runtime path

Clone the monorepo and add the plugin directory to Vim's runtime path:

```sh
git clone https://github.com/jarkkojs/readseek ~/.vim/pack/plugins/opt/readseek
```

```vim
set runtimepath^=~/.vim/pack/plugins/opt/readseek/packages/readseek.vim
```

Run `:ReadSeekCheckHealth` in Vim to verify the executable and version.

## Configuration

```vim

" Root marker search order. The nearest directory containing one is used.
let g:readseek_root_markers = ['.git']

" Use quickfix or location-list output. Defaults to 'quickfix'.
let g:readseek_list_type = 'quickfix'

" Optional: use an existing executable instead of the managed release binary.
let g:readseek_executable = '/path/to/readseek'
```

Available `<Plug>` mappings:

| Mapping                      | Command                |
|------------------------------|------------------------|
| `<Plug>(ReadSeekDefinition)` | `:ReadSeekDefinition`  |
| `<Plug>(ReadSeekReferences)` | `:ReadSeekReferences`  |
| `<Plug>(ReadSeekRename)`     | `:ReadSeekRename`      |
| `<Plug>(ReadSeekHover)`      | `:ReadSeekHover`       |
| `<Plug>(ReadSeekSearch)`     | `:ReadSeekSearch`      |
| `<Plug>(ReadSeekMap)`        | `:ReadSeekMap`         |
| `<Plug>(ReadSeekInit)`       | `:ReadSeekInit`        |

Define your preferred keys in vimrc:

```vim
nnoremap <silent> gd <Plug>(ReadSeekDefinition)
nnoremap <silent> gr <Plug>(ReadSeekReferences)
nnoremap <silent> K <Plug>(ReadSeekHover)
nnoremap <silent> ,rn <Plug>(ReadSeekRename)
nnoremap <silent> ,rs <Plug>(ReadSeekSearch)
nnoremap <silent> ,rm <Plug>(ReadSeekMap)
```

## Commands

- `:ReadSeekCheckHealth` checks executable discovery and `readseek` version.
- `:ReadSeekHover` shows identifier context at the cursor.
- `:ReadSeekDefinition` jumps to one definition or opens quickfix for multiple.
- `:ReadSeekReferences` opens quickfix with references for the cursor identifier.
- `:ReadSeekRename` prompts for a new name and applies a binding-accurate rename
  to the current (saved) file via `readseek rename --apply`.
- `:ReadSeekMap` maps the current buffer to a symbol outline in the results list.
- `:ReadSeekInit` initializes the `readseek` cache for the current project root.

## Tests

Run the lightweight Vim script test suite with:

```sh
vim -Nu NONE -n -i NONE -es -S test/readseek.vim
```

See `:help readseek` for detailed behavior and troubleshooting.

## License

`readseek.vim` is licensed under `MIT`. See [LICENSE](LICENSE). The ReadSeek
binary it downloads is licensed under `LGPL-2.1-or-later`.
