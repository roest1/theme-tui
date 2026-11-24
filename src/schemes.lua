-- Run by `font` inside a headless nvim. Prints one line per colorscheme this
-- nvim has that it did not ship with: its name, then the colors of a handful
-- of highlight groups, tab-separated.
--
--   scheme <TAB> dracula <TAB> Normal=f8f8f2,282a36 <TAB> Comment=6272a4, ...
--
-- A color is six hex digits or nothing. See schemes.rs for the other end.

-- The config's own loader, when it has one. dotfiles' external/theme.lua
-- publishes `load(name, transparent)`: it is what the lane will be drawn by,
-- so the preview goes through it and nothing about a scheme is restated here.
local ok, theme = pcall(require, 'external.theme')
local load = ok and type(theme) == 'table' and theme.load
  or function(name)
    pcall(vim.cmd.colorscheme, name)
  end

local shipped = vim.env.VIMRUNTIME or ''
local names, seen = {}, {}
for _, file in ipairs(vim.api.nvim_get_runtime_file('colors/*', true)) do
  local name = vim.fn.fnamemodify(file, ':t:r')
  if file:sub(1, #shipped) ~= shipped and not seen[name] then
    seen[name] = true
    names[#names + 1] = name
  end
end
table.sort(names)

local GROUPS = {
  'Normal',
  'Comment',
  'String',
  'Keyword',
  'Function',
  'Type',
  'Number',
  'Identifier',
  'Operator',
  'Delimiter',
  'LineNr',
  'CursorLineNr',
  'CursorLine',
  'Visual',
  'StatusLine',
}

local function hex(n)
  return n and ('%06x'):format(n) or ''
end

for _, name in ipairs(names) do
  -- Opaque, so the scheme's own background is there to read. The picker draws
  -- a transparent lane by leaving it out.
  load(name, false)
  local out = { 'scheme', name }
  for _, group in ipairs(GROUPS) do
    local hl = vim.api.nvim_get_hl(0, { name = group, link = false })
    out[#out + 1] = group .. '=' .. hex(hl.fg) .. ',' .. hex(hl.bg)
  end
  io.stdout:write(table.concat(out, '\t'), '\n')
end
