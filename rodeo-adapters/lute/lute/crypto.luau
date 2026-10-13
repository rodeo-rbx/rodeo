-- @lute/crypto has no rodeo implementation: no host cryptography.
error("@lute/crypto is not supported by rodeo's Lute adapter", 0)
return nil -- unreachable; darklua bundles only modules that end with a return
