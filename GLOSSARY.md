# Nexus

The words this project uses for its parts, in plain terms. Code, docs and commits use these
words and not the ones listed under _Avoid_.

## Language

**Area**:
A part of the core's state that a change names: the torrents, the catalogs, the installs, the
library and so on.
_Avoid_: section, domain

**Capability**:
One thing a plugin can do for the host, such as list games, offer releases or install builds.
A plugin's manifest declares its capabilities.
_Avoid_: feature, extension point

**Change**:
The core's note that its state in one area changed. The core and its tests read changes; the
app renders on any of them.
_Avoid_: update, state event

**Clock**:
The time the core runs on: what time it is, and its waits. The app's clock is real time.
_Avoid_: timer, time source

**Fake clock**:
The clock in tests. Its time moves only when a test moves it, or when every task waits for a
timer.
_Avoid_: mock time, test time

**Kind**:
What sort of plugin a manifest says it is: emulator, service, theme or system. The kind decides
a plugin's privileges.
_Avoid_: type, category

**Port**:
What the core needs from the outside world, as one interface. The app plugs in real ones and
the tests plug in fakes.
_Avoid_: adapter (that is the thing plugged in), driver
