---
name: server_authority_convert
description: Refactor core gameplay scripts to be Server Authority compliant so they work with client side prediction.
user_invocable: true
---

## What to do

1. Connect to Studio — if multiple instances are open, call `list_roblox_studios` and `set_active_studio` first.

2. Read the rules before converting: `api_reference/3_core_rules.md` and `api_reference/4_anti_patterns.md` (especially the Violation → Fix Quick Reference table).

3. Audit the game first. Run the same steps as `server_authority_audit`: find the core gameplay mechanics and check each one against the compliance checklist, producing the report table of mechanics and their violations.

4. Present the report to the user and ask them to approve the set of changes before editing any scripts. Do not edit scripts until the user approves.

5. Apply the refactors ONE MECHANIC AT A TIME. Do not batch edits across mechanics — finish and verify one before starting the next. For each approved mechanic, in order:
   1. Apply the refactor using the fixes from `api_reference/4_anti_patterns.md`. Common conversions:
      - Move gameplay logic out of LocalScripts into a ModuleScript required by a Client script (RunContext = Client) and a Server script (RunContext = Server). See `code_samples/1_SimulationTemplate.luau`.
      - Replace `Heartbeat` / `RenderStepped` / `Stepped` update loops with `RunService:BindToSimulation()`.
      - Replace `UserInputService` / `ContextActionService` with `InputAction` reads under the Player.
      - Move dynamic gameplay state out of raw Luau variables into Attributes on predicted instances.
      - Guard simulation code so it only runs when `RunService:GetPredictionStatus(instance) ~= Enum.PredictionStatus.None`.
      - Wrap any `RunService:SetPredictionMode()` call in `task.defer()`.
   2. Re-run the audit checklist against the changed script and confirm the violation is resolved and nothing new was introduced.
   3. If the correct fix is ambiguous, or depends on a Group 2 judgment check (for example whether an instance is actually predicted), STOP and ask the user before continuing. Do not guess.

6. When all approved mechanics are done, report a summary: what is now compliant, what still needs the user's attention, and any mechanics you skipped and why.
