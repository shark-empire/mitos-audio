mitos-audio — Routing Rules

Rules decide where audio goes in response to system events: devicehotplug, application streams, profile changes. Rules live inrouting.toml (config key routing_path), reload at runtime viaReloadRouting / mitos-audioctl reload-routing, and their restore memoryis persisted across daemon restarts.

Rule structure

[[rule]]name    = "unique-name"        # required, used in logswhen    = "device-added"       # trigger (see table)match   = { kind = "headset" } # conditions, all must match (see table)stop    = false                # stop evaluation after this rule firesenabled = true                 # quick disable without deleting[[rule.action]]                # 1+ actions per rule, executed in ordertype    = "set-default-output"target  = "trigger"            # or a literal device idremember = true                # save previous default for restore
Triggers (when)

Trigger	Fires when
device-added	hardware scan discovers a new device (udev/poll)
device-removed	a device disappears (unplug, BT disconnect)
stream-added	an application registers a stream (CreateStream)
profile-changed	the active profile changes
Conditions (match) — glob patterns, case-insensitive (*, ?)

Field	Matches	Example
kind	device kind	"headset", "hdmi*"
bus	device bus	"bluetooth", "usb"
name	device display name	"HDAIntel*"
id	device id	"alsa:hw:1,*"
application	stream application (stream rules)	"mitos-*", "*game*"
profile	profile active at trigger time	"bluetooth-*"
Device conditions on a stream rule match the stream's current device,enabling device-scoped app routing. Conditions referencing absent context(e.g. application on a device trigger) simply don't match.

Actions

Action	Effect
set-default-output (target, remember)	target: "trigger" or a device id; invalid/would-be-no-op actions are skipped silently
set-default-input (target)	same, for the default input
set-profile (profile)	switch profile (validated)
move-stream (to)	move the stream that triggered the rule to device to
restore-output	default output ← remembered previous; memory cleared
restore-input	same, for input
remember = true stores the current default before switching, so a laterrestore-output returns to it. Restore memory survives restarts(state.json → routing).

Evaluation

Rules run in file order when a trigger fires; every matching rule'sactions execute (unless an earlier rule set stop = true). Failing actionsare skipped with a warning — one bad rule never blocks the rest. Rule-inducedchanges emit the normal events (DefaultOutputChanged, StreamChanged, …)and are persisted like manual ones.

Testing

mitos-audioctl reload-routing          # after editing rulesmitos-audioctl new-stream "My Game"    # fires stream-added rulesmitos-audioctl streams                 # verify the stream was movedmitos-audioctl monitor                 # watch events as rules fire
On real hardware: plug/unplug a USB headset — udev delivers the event within~400 ms (debounced), then device rules fire.