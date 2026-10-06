-- The studio backend that installed this plugin file bakes its WebSocket
-- port onto the rbxm root (`rodeo_plugin`, this script's great-grandparent).
-- One plugin file per running backend, so each plugin knows which backend is
-- its own. A rojo-served dev plugin has no attribute and takes the default.
local pluginRoot = script.Parent.Parent.Parent
local bakedPort = pluginRoot:GetAttribute("rodeoPort")

-- Default studio-backend WebSocket port (= rodeo-cli's SERVE_PORT + 1; keep in
-- sync with rodeo-cli/src/util/config.rs::SERVE_PORT).
local DEFAULT_PORT = 44873
local PORT: number = if type(bakedPort) == "number" then bakedPort else DEFAULT_PORT

-- This plugin instance's key in `_G.__rodeo_plugins`. Every plugin in a DOM
-- shares one Luau VM and one _G, so per-plugin state lives in a table keyed
-- by build and port instead of flat globals. The launch bootstrap computes the
-- same key in Rust (`plugin_key` in launch.rs) to hand cmdbar results back.
local PLUGIN_KEY = `{RODEO_BUILD_ID}-{PORT}`
-- This plugin's file in Studio's plugins folder, as the backend names it
-- (`plugin_file_name` in launch.rs). A rojo-served dev plugin has no file.
local PLUGIN_FILE = `rodeo-{RODEO_BUILD_ID}-{PORT}.rbxm`

return {
	-- Suffix used to rename modules during uncacheable require traversal
	IGNORE_SUFFIX = ".rodeo.ignore.",

	-- Rodeo version, injected at build time by darklua (inject_global_value).
	VERSION = RODEO_VERSION,
	-- Build id (`version[+git sha]`), injected the same way. Sent on the WS
	-- handshake as StudioStateMsg.pluginVersion; the backend refuses a DOM
	-- whose id differs from its own, and reports its own in WelcomeMsg.version.
	BUILD_ID = RODEO_BUILD_ID,

	DEFAULT_HOST = "localhost",
	DEFAULT_PORT = DEFAULT_PORT,
	-- This plugin's own studio-backend port: baked into the plugin file by the
	-- backend that installed it, else the default. The plugin dials only this
	-- port. A rodeo launch's bootstrap stamps the launching backend's port as
	-- the `rodeoPort` Workspace attribute; a plugin whose PORT differs stays
	-- dormant in that Studio (see AppContext), and `rodeoSession` names the
	-- launch on the handshake.
	PORT = PORT,
	PLUGIN_KEY = PLUGIN_KEY,
	PLUGIN_FILE = PLUGIN_FILE,
	VERBOSE = false,

	-- Bit positions for context bitset
	-- Order: Server, Client, Running, Edit, Studio
	CONTEXT_BITS = {
		isServer = 0,
		isClient = 1,
		isRunning = 2,
		isEdit = 3,
		isStudio = 4,
	},

	-- UI Colors for arcade design - Rodeo red theme
	COLORS = {
		-- Backgrounds - dark with subtle red tint
		background = Color3.fromRGB(8, 6, 5),
		backgroundLight = Color3.fromRGB(20, 10, 12),

		-- Text - red/pink tones
		text = Color3.fromRGB(255, 150, 160),
		textDim = Color3.fromRGB(180, 100, 110),

		-- Accents - vibrant crimson (logo red)
		accent = Color3.fromRGB(255, 70, 80),
		accentHover = Color3.fromRGB(255, 100, 110),

		-- Status colors - green for connected, red for disconnected, yellow for executing
		success = Color3.fromRGB(140, 220, 100),
		error = Color3.fromRGB(220, 80, 90),
		executing = Color3.fromRGB(255, 200, 50),

		-- Button colors - bright crimson for connect, vibrant red-orange for disconnect
		buttonConnect = Color3.fromRGB(255, 70, 80),
		buttonDisconnect = Color3.fromRGB(255, 90, 70),

		-- Component-specific
		buttonBackground = Color3.fromRGB(35, 20, 22),
		buttonHover = Color3.fromRGB(45, 28, 30),
		border = Color3.fromRGB(120, 60, 65),
		inputBackground = Color3.fromRGB(20, 12, 14),
		stroke = Color3.fromRGB(200, 90, 100),
	},
}
