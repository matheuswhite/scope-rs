coroutine.create(function(t)
  if t == nil then
    return M.{}()
  else
    return M.{}(table.unpack(t))
  end
end)
