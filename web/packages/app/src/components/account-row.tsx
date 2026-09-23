import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { Icon } from "@zeron/icons";
import { useEngineSessions } from "../state/session-provider";
import { useFleet } from "../state/fleet";
import { useWatchSnapshot } from "../state/hooks";
import { PickerCard } from "./ui/PickerCard";

/**
 * The sidebar's bottom identity control — the desktop's `render_user_menu`.
 *
 * Upstream f9563394 compacts it: the row's label and subline collapse into
 * a 21px circular avatar button carrying the name's initial (13px white
 * circle, 9px mono semibold) with the name in the aria label — the menu
 * opens to the RIGHT of the trigger (the desktop's
 * `popover::anchored_menu_right`, shell.rs:6430 — the card's top-left pins
 * at the trigger's top-right + 6, clamped into the window).
 *
 * Ticket 02: the card rides `PickerCard`'s `anchorRight` variant — the
 * Base UI body portal. The old inline `.user-menu-card` (absolute, `left:
 * calc(100% + 6px)`) painted inside `<aside class="sidebar">` whose
 * `overflow: hidden` clipped it entirely, so the press looked dead while
 * the state and navigation worked. The portal puts the card on the menu
 * tier at the body, unclipped by any column; outside presses and Escape
 * dismiss through Base UI's pipeline (the old hand-rolled window
 * listeners are gone), and the trigger's press toggles with the
 * `trigger-press` reason exactly like every other anchored menu.
 *
 * The menu carries exactly two things — desktop parity (`shell.rs`): the
 * muted "Stored on this device" identity line, then the single "Settings"
 * row, which lands on the Devices section (`SettingsSection::Devices`),
 * the desktop's landing row. Engine management lives in Settings →
 * Devices (ticket 45 folded the old web-only Engines drawer there).
 */
export function AccountRow() {
  // The ACTIVE engine's session carries this row's identity — the desktop's
  // user line is the local device, and the web's "home" engine is the one
  // new chats land on.
  const sessions = useEngineSessions();
  const fleet = useFleet();
  const session = fleet.active === null ? null : sessions.get(fleet.active) ?? null;
  const snapshot = useWatchSnapshot(session);
  const navigate = useNavigate();
  // Controlled by the trigger's press — every dismissal (outside press,
  // Escape, the Settings row's navigation) lands here as `false`.
  const [open, setOpen] = useState(false);

  // The connected engine's own device is the identity this row carries — the
  // desktop's user line is the local device, not the transport. Falls back to
  // the engine label while the devices stream is still filling; the subline
  // is gone (f9563394) — the name rides the trigger's aria label alone.
  const engine = fleet.engines.find((candidate) => candidate.baseUrl === fleet.active) ?? null;
  const deviceId = session?.client.engineInfo?.deviceId ?? null;
  const device =
    deviceId === null ? undefined : snapshot?.devices.rows.find((row) => row.id === deviceId);
  const name = device?.name ?? engine?.label ?? "Zeron";
  const initial = (name.trim()[0] ?? "?").toUpperCase();

  function goSettings(): void {
    setOpen(false);
    void navigate({ to: "/settings/devices" });
  }

  return (
    <div className="user-menu">
      {/* The frame is the shared `.popover-card` glass; `.user-menu-body`
          carries only the card-content specifics (the 180px floor and the
          1px row rhythm the old card had). `anchorRight` is the desktop's
          `anchored_menu_right` placement, portaled by `PickerCard`. */}
      <PickerCard
        open={open}
        onOpenChange={setOpen}
        placement="anchorRight"
        cardClassName="popover-card user-menu-body"
        role="menu"
        ariaLabel="Account menu"
        initialFocus={false}
        trigger={
          <button
            type="button"
            className={`user-menu-trigger ${open ? "user-menu-trigger-open" : ""}`}
            aria-label={`Account menu: ${name}`}
            aria-haspopup="menu"
          >
            <span className="avatar" aria-hidden="true">
              {initial}
            </span>
          </button>
        }
      >
        <div className="user-menu-identity">Stored on this device</div>
        <button type="button" className="menu-item" role="menuitem" onClick={goSettings}>
          <Icon name="settingsMinimalistic" size={16} />
          Settings
        </button>
      </PickerCard>
    </div>
  );
}
