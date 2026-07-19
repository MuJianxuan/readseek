" SPDX-License-Identifier: MIT
" Copyright (c) 2026 Jarkko Sakkinen

vim9script

if exists('g:loaded_readseek')
  finish
endif

if !has('vim9script') || !has('job') || !has('channel') || !has('popupwin') || !has('textprop')
  echoerr 'readseek.vim requires Vim9 with +job, +channel, +popupwin, and +textprop'
  finish
endif


import autoload 'readseek/config.vim'
import autoload 'readseek/install.vim'

try
  config.Initialize()
catch
  echoerr v:exception
  finish
endtry

g:loaded_readseek = true

command! ReadSeekCheckHealth readseek#CheckHealth()
command! ReadSeekHover readseek#Hover()
command! ReadSeekDefinition readseek#Definition()
command! ReadSeekReferences readseek#References()
command! ReadSeekRename readseek#Rename()
command! ReadSeekSearch readseek#Search()
command! ReadSeekMap readseek#Map()
command! ReadSeekCheck readseek#Check()
command! -count=21 ReadSeekRead readseek#Read(<count>)
command! ReadSeekSymbol readseek#Symbol()
command! ReadSeekDetect readseek#Detect()
command! ReadSeekInit readseek#Init()
command! ReadSeekInstall install.Install((result: dict<any>) => readseek#InstallComplete(result), false)
command! ReadSeekUpdate install.Install((result: dict<any>) => readseek#InstallComplete(result), true)
command! ReadSeekUninstall install.Uninstall((result: dict<any>) => readseek#InstallComplete(result))

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
MapPlugDefault('<Plug>(ReadSeekCheck)', '<ScriptCmd>ReadSeekCheck<CR>')
MapPlugDefault('<Plug>(ReadSeekRead)', '<ScriptCmd>ReadSeekRead<CR>')
MapPlugDefault('<Plug>(ReadSeekSymbol)', '<ScriptCmd>ReadSeekSymbol<CR>')
MapPlugDefault('<Plug>(ReadSeekDetect)', '<ScriptCmd>ReadSeekDetect<CR>')
MapPlugDefault('<Plug>(ReadSeekInit)', '<ScriptCmd>ReadSeekInit<CR>')

highlight default ReadSeekOk ctermfg=green guifg=#00d700
highlight default ReadSeekInfo ctermfg=blue guifg=#5f87af
highlight default ReadSeekWarn ctermfg=yellow guifg=#d7d700
highlight default ReadSeekError ctermfg=red guifg=#d70000
highlight default ReadSeekBorder ctermfg=blue guifg=#5f87af
highlight default ReadSeekTitle cterm=bold ctermfg=blue gui=bold guifg=#5f87af
highlight default link ReadSeekFloat Normal

if config.AutoInstall() && empty(get(g:, 'readseek_executable', '')) && !config.IsExecutableAvailable()
  install.Install((result: dict<any>) => readseek#InstallComplete(result), false)
endif