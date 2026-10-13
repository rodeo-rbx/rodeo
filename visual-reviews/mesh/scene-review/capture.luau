-- @rodeo run --adapter lune
local r = require("@rodeo/roblox")
local fs = require("@lune/fs")
local serde = require("@lune/serde")
local phase = require("@rodeo/process").args[1]
local directory = "visual-reviews/mesh/scene-review/"
local lighting = game:GetService("Lighting")
lighting.ClockTime = 14
lighting.Brightness = 2
lighting.Ambient = Color3.fromRGB(100,100,100)
lighting.OutdoorAmbient = Color3.fromRGB(128,128,128)
workspace.Terrain:Clear()
for _, item in workspace:GetChildren() do if item:IsA("BasePart") and not item:IsA("Terrain") then item:Destroy() end end
for _, fixture in {"Box", "BoxTextured", "AnimatedMorphCube", "animated-skin"} do
 local path = if fixture == "animated-skin" then "tests/fixtures/pkg/scenes/animated-skin.gltf" else `tests/fixtures/pkg/scenes/khronos/{fixture}.glb`
 local scene = r.importEditableScene(path)
 for _, root in scene.roots do root.Parent = workspace end
 local track
 if fixture == "animated-skin" then
  track = scene.animator:LoadAnimation(scene.animations[1].animation)
  local deadline=os.clock()+5;while track.Length==0 and os.clock()<deadline do task.wait() end
  assert(track.Length>0);track:Play(0,1,0);track.TimePosition=.5;scene.animator:StepAnimations(0)
 end
 local cameraPath = `{directory}{fixture}-camera.json`
 local cf
 if phase == "baseline" then
  local low,high = Vector3.one*math.huge,-Vector3.one*math.huge
  for _, binding in scene.sourceMap.primitives do
   local part = binding.part
   for x=-1,1,2 do for y=-1,1,2 do for z=-1,1,2 do
    local p=part.CFrame:PointToWorldSpace(Vector3.new(x,y,z)*part.Size*.5)
    low=low:Min(p);high=high:Max(p)
   end end end
  end
  local center=(low+high)*.5;local distance=math.max((high-low).Magnitude*1.4,3)
  cf=CFrame.lookAt(center+Vector3.new(.8,.5,1).Unit*distance,center)
  if fixture == "animated-skin" then local target=Vector3.new(0,1.5,0);cf=CFrame.lookAt(target+Vector3.new(.8,.5,1).Unit*10,target) end
  fs.writeFile(cameraPath,serde.encode("json",{cf:GetComponents()}))
 else cf=CFrame.new(unpack(serde.decode("json",fs.readFile(cameraPath)))) end
 r.captureViewport(`{directory}{fixture}-{phase}.png`,{cframe=cf,fov=45,viewportSize=Vector2.new(1000,700),settle=.5})
 if track then track:Stop(0);track:Destroy() end
 for _, root in scene.roots do root:Destroy() end
 for _, mesh in scene.meshes do mesh:Destroy() end
 for _, image in scene.images do image:Destroy() end
end
return phase
