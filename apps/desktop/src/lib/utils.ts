import { clsx, type ClassValue } from "clsx";
import { extendTailwindMerge } from "tailwind-merge";

/**
 * tailwind-merge configured for the blurrd text scale: `text-ui-*` sizes
 * are font sizes, so a later `text-ui-sm` replaces an earlier `text-ui-base`
 * instead of being mistaken for a text colour and kept side by side.
 */
const merge = extendTailwindMerge({
  extend: {
    classGroups: {
      "font-size": [{ "text-ui": ["xl", "lg", "base", "caption", "sm", "xs", "2xs"] }],
    },
  },
});

/**
 * Joins class names (strings, arrays, conditional objects) and resolves
 * Tailwind conflicts so that the last class of a group wins.
 */
export function cn(...inputs: ClassValue[]): string {
  return merge(clsx(inputs));
}
