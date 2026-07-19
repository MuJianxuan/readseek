" SPDX-License-Identifier: MIT
" Copyright (c) 2026 Jarkko Sakkinen

vim9script

if exists('g:loaded_readseek')
  finish
endif
g:loaded_readseek = true

if !exists('g:readseek_root_markers')
  g:readseek_root_markers = ['.git']
endif
if !exists('g:readseek_list_type')
  g:readseek_list_type = 'quickfix'
endif

command! ReadSeekCheckHealth readseek#CheckHealth()
command! ReadSeekHover readseek#Hover()
command! ReadSeekDefinition readseek#Definition()
command! ReadSeekReferences readseek#References()
command! ReadSeekRename readseek#Rename()
command! ReadSeekSearch readseek#Search()
command! ReadSeekMap readseek#Map()
command! ReadSeekInit readseek#Init()

def MapPlugDefault(lhs: string, rhs: string)
  if !empty(maparg(lhs, 'n'))
    return
  endif
  execute $'nnoremap <silent> {lhs} {rhs}'
enddef

MapPlugDefault('<Plug>(ReadSeekDefinition)', '<ScriptCmd>ReadSeekDefinition<CR>')
MapPlugDefault('<Plug>(ReadSeekReferences)', '<ScriptCmd>ReadSeekReferences<CR>')
MapPlugDefault('<Plug>(ReadSeekHover)', '<ScriptCmd>ReadSeekHover<CR>')
MapPlugDefault('<Plug>(ReadSeekRename)', '<ScriptCmd>ReadSeekRename<CR>')
MapPlugDefault('<Plug>(ReadSeekSearch)', '<ScriptCmd>ReadSeekSearch<CR>')
MapPlugDefault('<Plug>(ReadSeekMap)', '<ScriptCmd>ReadSeekMap<CR>')
MapPlugDefault('<Plug>(ReadSeekInit)', '<ScriptCmd>ReadSeekInit<CR>')

highlight default ReadSeekOk ctermfg=green guifg=#00d700
highlight default ReadSeekInfo ctermfg=blue guifg=#5f87af
highlight default ReadSeekWarn ctermfg=yellow guifg=#d7d700
highlight default ReadSeekError ctermfg=red guifg=#d70000
highlight default ReadSeekBorder ctermfg=blue guifg=#5f87af
highlight default ReadSeekTitle cterm=bold ctermfg=blue gui=bold guifg=#5f87af
highlight default link ReadSeekFloat Normal

import autoload 'readseek/config.vim'
import autoload 'readseek/install.vim'

def OnAutoInstall(result: dict<any>)
enddef

if empty(get(g:, 'readseek_executable', '')) && (!config.IsExecutableAvailable() || config.Version() !=# config.PluginVersion)
  install.Install(OnAutoInstall)
endif
