import { expect, test } from "vitest";
import {
  axisControlKind,
  axisDefaultValue,
  axisOptions,
  normalizeAxisSelections,
  selectionIsValid
} from "./mediaAxisControls";

const enumAxis = {
  id: "aspect_ratio",
  label: "Aspect ratio",
  role: "param",
  control: { enum: { values: ["16:9", "9:16"], default: "16:9" } },
  requestField: "metadata.ratio",
  wireType: "string"
};

const rangeAxis = {
  id: "duration_seconds",
  label: "Duration",
  role: "param",
  control: { range: { min: 4, max: 12, step: 2, default: 6 } },
  requestField: "seconds",
  wireType: "number"
};

const boolAxis = {
  id: "audio",
  label: "Native audio",
  role: "selector",
  control: { bool: { default: true } },
  requestField: null,
  wireType: "string"
};

test("axis helpers expose enum metadata", () => {
  expect(axisControlKind(enumAxis)).toBe("enum");
  expect(axisOptions(enumAxis)).toEqual(["16:9", "9:16"]);
  expect(axisDefaultValue(enumAxis)).toBe("16:9");
  expect(selectionIsValid(enumAxis, "9:16")).toBe(true);
  expect(selectionIsValid(enumAxis, "1:1")).toBe(false);
});

test("axis helpers normalize bool and range values to strings", () => {
  expect(axisControlKind(rangeAxis)).toBe("range");
  expect(axisDefaultValue(rangeAxis)).toBe("6");
  expect(selectionIsValid(rangeAxis, "8")).toBe(true);
  expect(selectionIsValid(rangeAxis, "9")).toBe(false);
  expect(axisControlKind(boolAxis)).toBe("bool");
  expect(axisDefaultValue(boolAxis)).toBe("true");
  expect(selectionIsValid(boolAxis, "false")).toBe(true);
});

test("normalizeAxisSelections keeps valid saved values and drops stale keys", () => {
  expect(
    normalizeAxisSelections([enumAxis, rangeAxis, boolAxis], {
      aspect_ratio: "9:16",
      duration_seconds: "20",
      audio: "false",
      stale_video_option: "remove-me"
    })
  ).toEqual({
    aspect_ratio: "9:16",
    duration_seconds: "6",
    audio: "false"
  });
});

test("malformed controls are invalid so the modal can block saving", () => {
  const malformedAxis = {
    id: "resolution",
    label: "Resolution",
    role: "param",
    control: { enum: { values: [], default: "" } },
    requestField: "metadata.resolution",
    wireType: "string"
  };

  expect(axisControlKind(malformedAxis)).toBe("invalid");
  expect(axisDefaultValue(malformedAxis)).toBeNull();
  expect(axisOptions(malformedAxis)).toEqual([]);
  expect(selectionIsValid(malformedAxis, "720p")).toBe(false);
});
