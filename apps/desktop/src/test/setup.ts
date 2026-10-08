// Vitest setup: unmount rendered trees after every test (Testing Library
// cleans up automatically only when Vitest globals are enabled).
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

afterEach(cleanup);
