# Flatpak application distribution

Vocabulary for discussing flatpak-rs and its relationship to existing Flatpak.

## Language

**Flatpak 1**:
The existing Flatpak platform against which compatibility is evaluated.
_Avoid_: Legacy mode (when referring to the platform)

**Flatpak Next**:
The proposed next-generation Flatpak direction discussed by Adrian Vovk and Sebastian Wick at LAS 2026, rather than a finalized specification.
_Avoid_: Flatpak 2 specification

**flatpak-rs**:
This experimental project exploring the Flatpak Next direction.
_Avoid_: Official Flatpak Next implementation

**Collection identity**:
The identity of a distribution collection, preserved across its mirrors and distinct from an application's app ID. Distinct collections may distribute applications with the same app ID.
_Avoid_: Remote nickname, repository path

**Application identity**:
The combination of collection identity and app ID distinguishing applications distributed by different collections.
_Avoid_: App ID (when referring to the complete identity)

**Application variant**:
An application identity qualified by architecture and branch.
_Avoid_: Revision

**Installed revision**:
The exact artifact version selected for an installed application variant or runtime.
_Avoid_: Branch

**Installed selection**:
The pair of exact application and compatible runtime revisions selected for an application variant.
_Avoid_: Revision (when referring to the pair)

**Update**:
An explicit replacement of an existing application variant's installed selection, preserving its identity and persistent data.
_Avoid_: Reinstall, implicit upgrade

**Update plan**:
A proposed update identifying its target application variant, expected installed selection, and desired installed selection. It does not reserve or change the installation.
_Avoid_: Update job, reservation

**Runtime**:
A separately distributed execution environment required by an application.
_Avoid_: Host environment
