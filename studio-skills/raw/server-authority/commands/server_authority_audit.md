---
name: server_authority_audit
description: Audit all scripts in the game for Server Authority compliance and produce a report of core gameplay mechanics and their issues.
user_invocable: true
---

## What to do

1. Connect to Studio — if multiple instances are open, call `list_roblox_studios` and `set_active_studio` first.

2. Read the rules before auditing: `api_reference/3_core_rules.md` and `api_reference/4_anti_patterns.md` (especially the Violation → Fix Quick Reference table).

3. Find Core Gameplay Mechanics.
Examine all scripts for the core gameplay mechanics. Examples of core gameplay mechanics are operations on the Humanoid and HumanoidRootPart, physics operations, point scoring, health, etc.

Search for these terms:
- Writing to LocalPlayer, Humanoid, Character, HumanoidRootPart
- BasePart APIs: AssemblyLinearVelocity, ApplyImpulse, Touched, and others
- RemoteEvent APIs: FireServer, FireClient, FireAllClients
- Legacy input: UserInputService, ContextActionService
- Update signals: Heartbeat, RenderStepped, Stepped
- Timing: os.time(), tick()

4. Examine Each Mechanic Against This Checklist.
Work through both groups for every mechanic.

**Group 1 — Mechanical checks.** These are decidable from the script text; decide them yourself and report them with high confidence.
- [ ] No legacy input: no `UserInputService` / `ContextActionService` drives gameplay (input comes from an `InputAction` under the Player instead).
- [ ] No non-simulation update loops: gameplay is not updated in `Heartbeat` / `RenderStepped` / `Stepped`; it uses `RunService:BindToSimulation()`, `BasePart.Touched`, or `InputAction.StateChanged`.
- [ ] Timing uses `time()`, not `os.time()` / `tick()`.
- [ ] Any `RunService:SetPredictionMode()` call is wrapped in `task.defer()`.
- [ ] Gameplay logic lives in a ModuleScript run on BOTH client and server, not in a LocalScript.

**Group 2 — Judgment checks.** You often CANNOT decide these from the script alone. Do NOT assert pass/fail — mark them "Needs review" and state what a human must confirm.
- [ ] All dynamic state that affects the simulation is read from and written back to an Attribute on a predicted instance (no raw Luau variable holds gameplay state). Requires judging which data is dynamic and gameplay-critical.
- [ ] The instances it touches are actually predicted. Automatic prediction is runtime/proximity-based and cannot be confirmed from source — treat this as a candidate finding and recommend confirming at runtime with `RunService:GetPredictionStatus()`.
- [ ] It skips instances where `RunService:GetPredictionStatus(instance) == Enum.PredictionStatus.None`.
- [ ] It only reads/writes SimulationAccess properties and Attributes inside the simulation.

5. Produce the report as a table with one row per mechanic:

| Mechanic | Script (path) | Script type | Rule/anti-pattern violated | Symptom | Suggested fix | Confidence |
|---|---|---|---|---|---|---|

- Script type is one of: ModuleScript, Client Script, Server Script, LocalScript.
- Confidence is **High** for findings from a Group 1 (mechanical) check, and **Needs review** for findings that depend on a Group 2 (judgment) check.
- If a mechanic passes every check, list it with "Compliant" in the violation column.
- Use the Violation → Fix Quick Reference table in `api_reference/4_anti_patterns.md` to fill in the fix column.
