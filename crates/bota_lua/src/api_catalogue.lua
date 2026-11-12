-- Runs only in a fresh host, before any untrusted source is installed.
local globals, units, abilities = ...
local entries = {}
local function list(namespace, values)
    for name, value in pairs(values) do
        if type(name) == "string" and type(value) == "function" then
            entries[#entries + 1] = namespace .. "\t" .. name
        end
    end
end
list("global", globals)
list("unit", units)
list("ability", abilities)
for name, value in pairs(globals) do
    if name ~= "_G" and type(value) == "table" then
        list(name, value)
    elseif type(value) == "number" then
        entries[#entries + 1] = "constant\t" .. name
    end
end
if loadstring("goto done ::done:: return 1") then
    entries[#entries + 1] = "syntax\tgoto"
    entries[#entries + 1] = "syntax\t::"
end
table.sort(entries)
return table.concat(entries, "\n") .. "\n"
