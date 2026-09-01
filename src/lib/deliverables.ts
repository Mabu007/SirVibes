import type { Artifact } from "./types";

/** Finished video, as opposed to something on the way to one. */
const FINISHED_VIDEO = ["mp4", "mov", "m4v"];

/**
 * Folders whose contents are working files by convention. A render that lands
 * in `work/` is a step, not a result.
 */
const INTERNAL_FOLDERS = ["work", "tmp", "temp", "cache", "proxy", "intermediate"];

const extensionOf = (name: string) => name.split(".").pop()?.toLowerCase() ?? "";

/**
 * What the user asked for, out of everything a run happened to write.
 *
 * Making a captioned video leaves a transparent overlay, check frames, a cut
 * and a transcript behind it. None of those are the thing that was asked for,
 * and nobody should have to understand the pipeline to find their video. So
 * when a run produced a finished video, that is what is shown. When it did not,
 * nothing changes — every other kind of work still presents what it made.
 */
export function deliverables(found: Artifact[]): Artifact[] {
  const videos = found.filter((a) => FINISHED_VIDEO.includes(extensionOf(a.name)));
  if (!videos.length) return found;

  const isInternal = (a: Artifact) =>
    a.path
      .split("/")
      .slice(0, -1)
      .some((segment) => INTERNAL_FOLDERS.includes(segment.toLowerCase()));

  const finished = videos.filter((a) => !isInternal(a));
  // Everything landed somewhere working-looking: better to show the videos than
  // to show nothing at all.
  return finished.length ? finished : videos;
}
