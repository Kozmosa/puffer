export type AxisControlKind = "enum" | "range" | "bool" | "invalid";

type MediaCapabilityAxisLike = {
  id: string;
  control: unknown;
};

type EnumControl = { enum: { values: string[]; default: string } };
type RangeControl = { range: { min: number; max: number; step: number; default: number } };
type BoolControl = { bool: { default: boolean } };

export function axisControlKind(axis: Pick<MediaCapabilityAxisLike, "control">): AxisControlKind {
  if (enumControl(axis)) return "enum";
  if (rangeControl(axis)) return "range";
  if (boolControl(axis)) return "bool";
  return "invalid";
}

export function axisOptions(axis: Pick<MediaCapabilityAxisLike, "control">): string[] {
  const control = enumControl(axis);
  return control ? [...control.enum.values] : [];
}

export function axisDefaultValue(axis: Pick<MediaCapabilityAxisLike, "control">): string | null {
  const enumValue = enumControl(axis);
  if (enumValue) return enumValue.enum.default;
  const rangeValue = rangeControl(axis);
  if (rangeValue) return String(rangeValue.range.default);
  const boolValue = boolControl(axis);
  if (boolValue) return boolValue.bool.default ? "true" : "false";
  return null;
}

export function selectionIsValid(
  axis: Pick<MediaCapabilityAxisLike, "control">,
  value: string | undefined
): value is string {
  if (value === undefined) return false;
  const enumValue = enumControl(axis);
  if (enumValue) return enumValue.enum.values.includes(value);
  const boolValue = boolControl(axis);
  if (boolValue) return value === "true" || value === "false";
  const rangeValue = rangeControl(axis);
  if (!rangeValue) return false;
  const numeric = Number(value);
  const { min, max, step } = rangeValue.range;
  if (!Number.isFinite(numeric) || numeric < min || numeric > max) return false;
  const offset = (numeric - min) / step;
  return Math.abs(offset - Math.round(offset)) < 1e-9;
}

export function normalizeAxisSelections(
  axes: MediaCapabilityAxisLike[],
  saved: Record<string, string>
): Record<string, string> {
  const next: Record<string, string> = {};
  for (const axis of axes) {
    if (selectionIsValid(axis, saved[axis.id])) {
      next[axis.id] = saved[axis.id];
      continue;
    }
    const defaultValue = axisDefaultValue(axis);
    if (defaultValue !== null && selectionIsValid(axis, defaultValue)) {
      next[axis.id] = defaultValue;
    }
  }
  return next;
}

export function capabilityAxesError(axes: MediaCapabilityAxisLike[]): string | null {
  if (!Array.isArray(axes)) return "Capability axes are malformed.";
  for (const axis of axes) {
    if (!axis.id || axisControlKind(axis) === "invalid") {
      return `Capability axis ${axis.id || "(missing id)"} is malformed.`;
    }
  }
  return null;
}

function enumControl(axis: Pick<MediaCapabilityAxisLike, "control">): EnumControl | null {
  const control = axis.control as Partial<EnumControl> | null | undefined;
  const enumValue = control?.enum;
  if (!enumValue || !Array.isArray(enumValue.values)) return null;
  if (enumValue.values.length === 0) return null;
  if (!enumValue.values.every((value) => typeof value === "string" && value.length > 0)) {
    return null;
  }
  if (typeof enumValue.default !== "string" || !enumValue.values.includes(enumValue.default)) {
    return null;
  }
  return { enum: { values: enumValue.values, default: enumValue.default } };
}

function rangeControl(axis: Pick<MediaCapabilityAxisLike, "control">): RangeControl | null {
  const control = axis.control as Partial<RangeControl> | null | undefined;
  const rangeValue = control?.range;
  if (!rangeValue) return null;
  const { min, max, step, default: defaultValue } = rangeValue;
  if (![min, max, step, defaultValue].every(Number.isFinite)) return null;
  if (max < min || step <= 0 || defaultValue < min || defaultValue > max) return null;
  return { range: { min, max, step, default: defaultValue } };
}

function boolControl(axis: Pick<MediaCapabilityAxisLike, "control">): BoolControl | null {
  const control = axis.control as Partial<BoolControl> | null | undefined;
  const boolValue = control?.bool;
  if (!boolValue || typeof boolValue.default !== "boolean") return null;
  return { bool: { default: boolValue.default } };
}
