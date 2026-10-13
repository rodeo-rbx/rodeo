-- @lute/syntax/parser has no rodeo implementation: it binds Lute's native parser.
error("@lute/syntax/parser is not supported by rodeo's Lute adapter", 0)
return nil -- unreachable; darklua bundles only modules that end with a return
