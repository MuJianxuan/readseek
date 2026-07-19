" SPDX-License-Identifier: MIT
" Copyright (c) 2026 Jarkko Sakkinen

vim9script

export def SetLocations(locations: list<any>, title: string)
  var items = ToItems(locations)
  var source_window = CurrentWindow()
  if get(g:, 'readseek_list_type', 'quickfix') ==# 'location'
    setloclist(0, [], 'r', {title: title, items: items})
    lopen
    RestoreWindow(source_window)
    return
  endif

  setqflist([], 'r', {title: title, items: items})
  copen
  RestoreWindow(source_window)
enddef

export def ToItems(locations: list<any>): list<dict<any>>
  var items: list<dict<any>> = []
  for location in locations
    add(items, {
      filename: get(location, 'file', ''),
      lnum: get(location, 'line', 1),
      col: get(location, 'column', 1),
      text: ItemText(location),
    })
  endfor
  return items
enddef

def ItemText(location: dict<any>): string
  var text = get(location, 'text', '')
  var kind = get(location, 'kind', '')
  var name = get(location, 'name', '')
  if empty(kind) || empty(name)
    return text
  endif

  return $'[{kind}] {name}: {text}'
enddef

def CurrentWindow(): number
  if !exists('*win_getid') || !exists('*win_gotoid')
    return 0
  endif

  return win_getid()
enddef

def RestoreWindow(window: number)
  if window <= 0 || !exists('*win_gotoid')
    return
  endif

  win_gotoid(window)
enddef
