import { NOT_REPLAYED_NOTE } from '../wording';

/** What this page does and does not claim, and the caveats that were not ranked first. Nothing here is dropped from the page. */
export function AboutSection({ caveats }: { caveats: readonly string[] }) {
  return (
    <section aria-labelledby="about-title">
      <h2 id="about-title">About this page</h2>
      <p className="muted">{NOT_REPLAYED_NOTE}</p>
      {caveats.map((c) => (
        <p key={c} className="note">
          {c}
        </p>
      ))}
    </section>
  );
}
