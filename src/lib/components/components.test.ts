import { describe, expect, it, vi } from "vitest";
import { fireEvent, render } from "@testing-library/svelte";
import StatusChip from "./StatusChip.svelte";
import Toggle from "./Toggle.svelte";
import Segmented from "./Segmented.svelte";
import ZoneBand from "./ZoneBand.svelte";
import ErrorBanner from "./ErrorBanner.svelte";

describe("components", () => {
  it("StatusChip announces its label", () => {
    const { getByRole } = render(StatusChip, { status: { label: "Connected", tone: "ok" } });
    expect(getByRole("status").textContent).toContain("Connected");
  });

  it("Toggle is a labelled switch", async () => {
    const onchange = vi.fn();
    const { getByRole } = render(Toggle, { label: "Start with Windows", checked: false, onchange });
    const sw = getByRole("switch", { name: "Start with Windows" });
    expect(sw.getAttribute("aria-checked")).toBe("false");
    await fireEvent.click(sw);
    expect(onchange).toHaveBeenCalledWith(true);
  });

  it("Segmented supports arrow keys", async () => {
    const onchange = vi.fn();
    const { getAllByRole } = render(Segmented, {
      label: "Range",
      value: "60s",
      options: [
        { value: "60s", label: "60 s" },
        { value: "5m", label: "5 min" },
      ],
      onchange,
    });
    const radios = getAllByRole("radio");
    expect(radios[0].getAttribute("aria-checked")).toBe("true");
    expect(radios[1].getAttribute("tabindex")).toBe("-1");
    await fireEvent.keyDown(radios[0], { key: "ArrowRight" });
    expect(onchange).toHaveBeenCalledWith("5m");
  });

  it("ZoneBand shows the current zone", () => {
    const { getByRole, getByText } = render(ZoneBand, {
      bpm: 150,
      maxHr: 190,
      thresholds: [50, 60, 70, 80, 90],
      zone: 3,
      active: true,
    });
    expect(getByRole("meter").getAttribute("aria-valuenow")).toBe("3");
    expect(getByText(/Aerobic/)).toBeTruthy();
  });

  it("ErrorBanner shows title, message and fix", () => {
    const { getByRole } = render(ErrorBanner, {
      error: { code: "radio_off", title: "Bluetooth is turned off", message: "Radio off.", fix: "Turn it on." },
    });
    const t = getByRole("alert").textContent ?? "";
    expect(t).toContain("Bluetooth is turned off");
    expect(t).toContain("Turn it on.");
  });
});
