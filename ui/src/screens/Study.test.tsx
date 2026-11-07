import { TooltipProvider } from "@vitavision/ui";
import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { api, type StudyResult } from "../api";
import fixture from "../test/study.fixture.json";
import { Study } from "./Study";

const result = fixture as unknown as StudyResult;

afterEach(() => vi.restoreAllMocks());

describe("Study", () => {
  it("runs the study and shows its frontier", async () => {
    vi.spyOn(api, "studySchema").mockResolvedValue({
      type: "object",
      properties: { description: { type: "string" } },
    });
    vi.spyOn(api, "studyExample").mockResolvedValue({ description: "x" });
    const run = vi.spyOn(api, "study").mockResolvedValue(result);
    render(
      <TooltipProvider>
        <Study captures={[]} onCreated={() => undefined} />
      </TooltipProvider>,
    );
    const button = await screen.findByRole("button", { name: /Run study/ });
    await screen.findByText("Study spec");
    fireEvent.click(button);
    expect((await screen.findAllByText("Pareto-optimal")).length).toBeGreaterThan(0);
    expect(run).toHaveBeenCalledWith({ description: "x" }, null);
    expect(screen.getByRole("img", { name: "Study runs" })).toBeInTheDocument();
    expect(screen.getAllByText("best").length).toBe(1);
    expect(screen.getByText(`Run ${result.best}`)).toBeInTheDocument();
  });
});
