# Desktop styling and interaction review

[← Slint interface](SLINT_UI.md)

Reviewed on 2026-10-30 against the current source and rendered Cupertino,
Fluent and Cosmic styles. This review covers the main file window, confirmation
forms, Settings, focus, text editing and action presentation. It does not certify
the app as a native AppKit, WinUI or GTK application: most controls are rendered by
Slint, with native window frames, menus and other integrations where documented.
The macOS Extract form now uses a real AppKit sheet; see the
[native form integration review](NATIVE_FORMS.md).

## Platform conventions

| Area | macOS | Windows | Linux |
| --- | --- | --- | --- |
| Widget family | Cupertino, with rounded form fields and push buttons | Standard Fluent fields, buttons and choices | Standard Cosmic fields, buttons and choices |
| Short action dialogs | Compact sheet-like surface; normal-height controls | Larger Fluent control metrics; inline validation retained | Larger desktop control metrics; inline validation retained |
| Footer order | Cancel before the action | Actions before Cancel/Close | Cancel before the action, following GNOME guidance |
| Main chrome | Existing neutral macOS surfaces; 6 px control corners | Neutral Windows surfaces; 4 px control corners | Neutral Linux surfaces; 8 px control corners |
| Keyboard | Command labels, native menus, Tab/Shift+Tab and text editing | Control labels, F2 rename and clipboard shortcuts | Control labels, keyboard navigation and clipboard shortcuts |
| Theme | System/light/dark preference | System/light/dark preference | System/light/dark preference; Ubuntu accent detection |

The review found ordinary form rows stretching to fill spare vertical space.
That enlarged single-line inputs and drop-downs without improving usability.
Forms now align rows at the top and keep each control's normal height. Compact
confirmations use field-aware height estimates and large forms keep a scrolling
body above the footer. Focus rings have breathing room; unrelated text labels
are not keyboard stops.

Windows no longer inherits the macOS/GNOME dismissal order. Button callbacks
still use the original command IDs after reordering. All styles use consistent
8 px button gaps and explicit action labels such as Copy, Move and Extract.
Safe confirmation actions can receive default emphasis; delete/trash prompts
do not receive that emphasis. Existing cancellation and confirmation guards
remain in effect. The new macOS fields retain selection, clipboard, Undo/Redo,
context-menu editing and disabled states; password accessibility values are
excluded.

The main window remains a dual-pane file manager. Commander function keys,
Space/Insert marks and the red marked-file appearance are intentional product
behavior. Active/cursor highlighting and accessibility selection state distinguish
focus from marks. Toolbar icons have accessible labels and keyboard focus;
file panes, locations, filtering and the tree retain keyboard navigation.

## Remaining acceptance and integration limits

- The Windows and Linux visual previews use their compiled widget styles on the
  development Mac. Linux container checks establish compilation, not a physical
  Windows/Linux desktop result. Native frame, screen-reader, compositor and
  mixed-DPI monitor testing still require those desktops.
- Cosmic is the current Linux fallback. It does not reproduce Adwaita, KDE
  Breeze or arbitrary installed GTK themes. GNOME action-dialog header bars are
  not implemented; this app retains its shared scrolling-form/footer structure.
- Windows High Contrast and Linux high-contrast theme integration are not
  complete. The custom file-manager chrome uses application color tokens rather
  than every platform accessibility/system color. Larger font preferences and
  logical-pixel scaling exist, but do not establish high-contrast acceptance.
- Windows/Linux menu access-key mnemonics and complete native menu-bar keyboard
  conventions remain incomplete. Existing configurable shortcuts, Ctrl+F6 focus
  traversal and Shift+F10 file context menus do not establish full Alt-key menu
  parity. Broad shortcut changes should preserve the established Commander
  bindings and user customizations.

These are recorded gaps, not reasons to replace functioning file operations or
force macOS styling onto the other desktops. See [validation](../parity/VALIDATION.md)
for the checks performed with this change.

## Design sources

- Apple: [Text fields](https://developer.apple.com/design/human-interface-guidelines/text-fields),
  [Buttons](https://developer.apple.com/design/human-interface-guidelines/buttons)
  and [Sheets](https://developer.apple.com/design/human-interface-guidelines/sheets).
- Microsoft: [Design guidelines](https://learn.microsoft.com/en-us/windows/apps/design/guidelines-overview),
  [Dialog controls](https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/dialogs-and-flyouts/dialogs),
  [Content layout and spacing](https://learn.microsoft.com/en-us/windows/apps/design/basics/content-basics)
  and [Keyboard interactions](https://learn.microsoft.com/en-us/windows/apps/develop/input/keyboard-interactions).
- GNOME: [Dialogs](https://developer.gnome.org/hig/patterns/feedback/dialogs.html),
  [UI styling](https://developer.gnome.org/hig/guidelines/ui-styling.html)
  and [Keyboard](https://developer.gnome.org/hig/guidelines/keyboard.html).
