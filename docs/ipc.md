mitos-audio IPC Protocol Reference

Transport & framing

Unix domain socket, default /run/mitos/audio.sock
UTF-8 JSON, one message per line (\n terminated)
Max line length: 1 MiB
Message shapes

Request (client → daemon):

{ "id": 1, "command": "SetVolume", "params": { "volume": 60 } }
Response (daemon → client):

{ "id": 1, "ok": true, "result": { "volume": 60 } }{ "id": 2, "ok": false, "error": { "code": "device-not-found", "message": "device not found: foo" } }
Event (daemon → subscribed clients):

{ "event": "VolumeChanged", "data": { "device": null, "volume": 60 } }
Responses are sent in request order per connection. On a subscribedconnection, events interleave with responses — distinguish by the presenceof "id" (response) vs "event" (event).

Commands

Command	Params	Result
Ping	–	{ "pong": true }
GetState	–	full snapshot (devices, streams, defaults, volumes, mic, profile, version)
GetDefaults	–	{ default_output, default_input }
ListDevices	–	{ devices: [...], default_output, default_input }
GetDevice	id	device object
SetDefaultOutput	id	{ default_output }
SetDefaultInput	id	{ default_input }
GetVolume	device?	{ volume, muted }
SetVolume	volume, device?	{ volume }
Mute / Unmute	–	{ muted }
ListStreams	–	{ streams: [...] }
SetStreamVolume	stream_id, volume	{ stream_id, volume }
SetStreamMute	stream_id, mute	{ stream_id, muted }
MoveStream	stream_id, device_id	{ stream_id, device }
ListProfiles	–	{ profiles: [...], active }
SetProfile	profile	{ profile }
GetMicrophone	–	{ device, muted, gain_db }
SetMicrophoneGain	gain (dB, clamped ±30)	{ gain_db }
MuteMicrophone	mute	{ muted }
GetLevels	–	{ master_volume, muted, output_level, input_level, peak, clipping }
SubscribeEvents	–	{ subscribed: true } then event lines
Example session
| Command | Params | Result |
|---------|--------|--------|
| `Rescan` | – | `{ devices: [...], default_output, default_input }` — forces a hardware rescan (also automatic every ~3 s) |



And append a "v0.2 semantics" block:

v0.2 semantics

Master volume = the default output device's volume. GetVolume /SetVolume / Mute / Unmute without a device parameter resolve tothe current default output; results and VolumeChanged events carry its id.
SetVolume / mute results include hw_applied (bool) — false when thedevice has no hardware volume control / mute switch (state still tracked).
The daemon rescans hardware every ~3 s and on Rescan. External changes(e.g. alsamixer, USB hotplug) surface as DeviceChanged /DeviceAdded / DeviceRemoved events.
GetState now includes "backend": "alsa" or "demo".
→ {"id":1,"command":"GetState","params":{}}← {"id":1,"ok":true,"result":{"service":"mitos-audio","version":"0.1.0", ... }}→ {"id":2,"command":"SetVolume","params":{"volume":60}}← {"id":2,"ok":true,"result":{"volume":60}}   (all subscribers also receive: {"event":"VolumeChanged","data":{"device":null,"volume":60}})→ {"id":3,"command":"MoveStream","params":{"stream_id":"s-1","device_id":"headphones"}}← {"id":3,"ok":true,"result":{"stream_id":"s-1","device":"headphones"}}
Events

Event	Data	Emitted when
DeviceAdded	{ id, name }	device hotplug
DeviceRemoved	{ id }	device removal
DeviceChanged	{ id }	state/profile change
DefaultOutputChanged	{ id }	default output switched
DefaultInputChanged	{ id }	default input switched
VolumeChanged	{ device?, volume }	master or per-device volume change
MuteChanged	{ muted }	master mute toggled
StreamAdded	{ id, application }	application opens a stream
StreamRemoved	{ id }	application closes a stream
StreamChanged	{ id }	stream volume/mute/device changed
ProfileChanged	{ profile }	profile activated
MicrophoneChanged	{ muted, gain }	mic mute or gain changed
Error codes

device-not-found, stream-not-found, invalid-volume, invalid-profile,not-an-output, not-an-input, ipc-error, config-error,config-parse-error, io-error, serialization-error

Testing

socat - UNIX-CONNECT:/run/mitos/audio.sock{"id":1,"command":"ListDevices","params":{}}mitos-audioctl monitor | jq .