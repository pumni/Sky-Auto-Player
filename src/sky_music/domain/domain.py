from dataclasses import dataclass
from typing import Literal, NewType

Millis = NewType("Millis", int)
ScanCode = NewType("ScanCode", int)
NoteKey = NewType("NoteKey", str)
Microseconds = NewType("Microseconds", int)

@dataclass(frozen=True, slots=True)
class Note:
    time_ms: Millis
    key: NoteKey

@dataclass(frozen=True, slots=True)
class Song:
    name: str
    notes: tuple[Note, ...]

@dataclass(frozen=True, slots=True)
class InstrumentProfile:
    name: str
    note_count: Literal[4, 8, 15]
    key_map: dict[NoteKey, str]
