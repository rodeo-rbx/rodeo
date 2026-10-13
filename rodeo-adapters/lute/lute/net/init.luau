-- @lute/net has no rodeo implementation: rodeo has no host networking.
error("@lute/net is not supported by rodeo's Lute adapter", 0)
return nil -- unreachable; darklua bundles only modules that end with a return
