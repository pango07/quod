import { describe, expect, it } from "vitest";
import { jsonbAsText } from "./postgres";

describe("postgres adapter", () => {
  it("casts jsonb parameters through text so the driver does not encode them twice", () => {
    expect(jsonbAsText("SELECT jsonb_array_elements_text($2::jsonb), $14::jsonb, $21::jsonb")).toBe(
      "SELECT jsonb_array_elements_text($2::text::jsonb), $14::text::jsonb, $21::text::jsonb",
    );
    expect(jsonbAsText("SELECT labels::jsonb, $3::jsonb_path, $1::timestamptz")).toBe("SELECT labels::jsonb, $3::jsonb_path, $1::timestamptz");
  });
});
