/**
 * FIRMWARE TOOLS (Milestone 14 Task 12), as the design draws it
 * (local/m14-design/Main.dc.html and Mobile.dc.html): the SPEED INDEX in
 * the header, PROVISIONING (the ROLE select, PROVISION and Skip setup)
 * and BADGE LINK (the room code, Host link and Join link, and the link
 * summary). It renders from the controller's snapshot and calls its
 * actions; it holds only the chosen role and the typed room code.
 *
 * - PROVISION is disabled while a provision or a skip runs (Milestone
 *   13's ruling T5-1). Skip setup is shown disabled, not hidden (the
 *   spec's call), and enabled only while Setup is on screen in this tab,
 *   no skip types and no provision is in flight (its token phase runs on
 *   Setup; ruling T4-1).
 * - **Cancel host.** While a hosted room waits, Host link reads "Cancel
 *   host" (the design's `.ghost.on`) and cancels it, which also closes the
 *   room on the server; the design has no other place for it.
 * - "Use phone motion" is shown only where DeviceMotion exists, below
 *   BADGE LINK, as a toggle (`aria-pressed`).
 *
 * {@link TabNotice} is the design's `tabNotice` banner.
 */
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { TEST_IDENTITY_ROLES } from "../runtime/test-identities";
import type { BadgeController, BadgeSnapshot } from "./badge-controller";

export type ToolsActions = Pick<
  BadgeController,
  "provision" | "skipSetup" | "host" | "cancelHost" | "join" | "usePhoneMotion"
>;

export interface ToolsPanelProps {
  snapshot: BadgeSnapshot;
  actions: ToolsActions;
}

/** A firmware role string (`workshop_lead`) as a menu label (`workshop lead`). */
export function roleLabel(role: string): string {
  return role.replace(/_/g, " ");
}

/** Which controls are enabled, from the snapshot. */
export function toolsState(s: BadgeSnapshot): { provisionEnabled: boolean; skipEnabled: boolean } {
  const provisionInFlight = s.provision?.kind === "waiting" || s.provision?.kind === "typing";
  const skipTyping = s.skip?.kind === "typing";
  return {
    provisionEnabled: !provisionInFlight && !skipTyping,
    skipEnabled: s.setupOnScreen && !provisionInFlight && !skipTyping,
  };
}

const SECTION_LABEL = "font-mono text-[10px] leading-none font-semibold tracking-[.12em] text-muted-foreground";

export function ToolsPanel({ snapshot, actions }: ToolsPanelProps) {
  const [role, setRole] = useState<string>(TEST_IDENTITY_ROLES[0]);
  const [code, setCode] = useState("");
  const { provisionEnabled, skipEnabled } = toolsState(snapshot);
  const phoneOn = snapshot.motion === "phone";

  return (
    <section className="flex flex-col overflow-hidden rounded-panel-mobile border border-border bg-card md:rounded-panel">
      <div className="flex flex-wrap items-baseline justify-between gap-3 border-b border-border px-4 py-3 md:px-[18px]">
        <h2 className="m-0 font-mono text-[10px] leading-none font-semibold tracking-[.14em] text-accent-text md:text-[11px]">
          FIRMWARE TOOLS
        </h2>
        <span className="font-mono text-xs leading-none font-medium text-muted-foreground">
          SPEED INDEX <span className="text-accent-text">{snapshot.speed.toFixed(1) + "x"}</span>
        </span>
      </div>
      <div className="flex flex-col gap-4 px-4 py-3.5 md:p-[18px]">
        <div role="group" aria-label="Provisioning" className="flex flex-col gap-2.5">
          <span className={SECTION_LABEL}>PROVISIONING</span>
          <div className="relative flex items-center">
            <span aria-hidden="true" className={`pointer-events-none absolute left-3 z-10 ${SECTION_LABEL}`}>
              ROLE
            </span>
            <Select value={role} onValueChange={setRole}>
              <SelectTrigger aria-label="Role" className="w-full pl-[58px]">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {TEST_IDENTITY_ROLES.map((r) => (
                  <SelectItem key={r} value={r}>
                    {roleLabel(r)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="grid grid-cols-2 gap-2">
            <Button type="button" disabled={!provisionEnabled} onClick={() => void actions.provision(role)}>
              PROVISION
            </Button>
            <Button type="button" disabled={!skipEnabled} onClick={() => actions.skipSetup()}>
              Skip setup
            </Button>
          </div>
        </div>
        <div role="group" aria-label="Badge link" className="flex flex-col gap-2.5 border-t border-border pt-3.5">
          <div className="flex min-w-0 items-baseline justify-between gap-3">
            <span className={SECTION_LABEL}>BADGE LINK</span>
            <span
              aria-live="polite"
              className="min-w-0 truncate font-mono text-[11px] leading-none font-medium text-accent-text"
            >
              {snapshot.linkStatus}
            </span>
          </div>
          <Input
            type="text"
            aria-label="Room code"
            placeholder="ROOM CODE"
            maxLength={16}
            autoComplete="off"
            spellCheck={false}
            value={code}
            onChange={(e) => setCode(e.target.value.toUpperCase().slice(0, 16))}
            className="uppercase"
          />
          <div className="grid grid-cols-2 gap-2">
            {snapshot.hostingRoom ? (
              <Button type="button" variant="on" onClick={() => actions.cancelHost()}>
                Cancel host
              </Button>
            ) : (
              <Button type="button" onClick={() => void actions.host()}>
                Host link
              </Button>
            )}
            <Button type="button" disabled={code.trim() === ""} onClick={() => void actions.join(code)}>
              Join link
            </Button>
          </div>
        </div>
        {snapshot.phoneMotionAvailable && (
          <div role="group" aria-label="Motion" className="flex flex-col gap-2.5 border-t border-border pt-3.5">
            <span className={SECTION_LABEL}>MOTION</span>
            <Button
              type="button"
              variant={phoneOn ? "on" : "default"}
              aria-pressed={phoneOn}
              onClick={() => void actions.usePhoneMotion(!phoneOn)}
            >
              Use phone motion
            </Button>
          </div>
        )}
      </div>
    </section>
  );
}

/** The design's `tabNotice`: another tab runs the badge, and Run here takes it over. */
export function TabNotice({ onRunHere }: { onRunHere: () => void }) {
  return (
    <div
      role="status"
      className="flex flex-wrap items-center justify-between gap-2.5 rounded-[10px] border border-notice-border bg-notice px-3.5 py-2.5 font-mono text-xs leading-[1.4] font-medium text-notice-foreground"
    >
      <span>Badge running in another tab</span>
      <Button type="button" onClick={onRunHere}>
        Run here
      </Button>
    </div>
  );
}
