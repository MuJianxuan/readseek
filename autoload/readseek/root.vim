" SPDX-License-Identifier: MIT
" Copyright (c) 2026 Jarkko Sakkinen

vim9script

import autoload 'readseek/buffer.vim'

export def Find(): string
  var path = buffer.Path()
  var dir = filereadable(path) ? fnamemodify(path, ':p:h') : getcwd()

  while !empty(dir)
    if HasMarker(dir)
      return dir
    endif

    var parent = fnamemodify(dir, ':h')
    if parent == dir
      break
    endif
    dir = parent
  endwhile

  return getcwd()
enddef

def HasMarker(dir: string): bool
  for marker in get(g:, 'readseek_root_markers', ['.git'])
    if isdirectory(dir .. '/' .. marker) || filereadable(dir .. '/' .. marker)
      return true
    endif
  endfor
  return false
enddef
