" SPDX-License-Identifier: MIT
" Copyright (c) 2026 Jarkko Sakkinen

vim9script

export def Stdin(): string
  var text = join(getline(1, '$'), "\n")
  if &endofline
    text ..= "\n"
  endif
  return text
enddef

export def Path(): string
  var path = expand('%:p')
  if !empty(path)
    return path
  endif
  return $'readseek-buffer-{bufnr('%')}'
enddef

export def Line(): number
  return line('.')
enddef

export def ByteColumn(): number
  return col('.')
enddef

export def IdentifyArgs(): list<string>
  return [
    'identify',
    $'stdin:{Path()}:{Line()}',
    '--column', string(ByteColumn()),
  ]
enddef
