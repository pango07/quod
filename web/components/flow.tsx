const STEPS = [
  {
    title: "Record",
    body: "Quod runs your tests and records the inputs used by each file, including shared configuration and loaded dependencies.",
  },
  {
    title: "Verify",
    body: "A fresh result needs confirmation before reuse. Quod compares test outcomes and checks the recorded inputs on later runs.",
  },
  {
    title: "Reuse",
    body: "Eligible files with unchanged inputs reuse their results. Changed and ineligible files run. Audits keep checking reused results.",
  },
];

/** Keep all three stages readable together without a client-side animation. */
export function Flow() {
  return (
    <ol className="flow">
      {STEPS.map((s, i) => (
        <li key={s.title} className="lit">
          <span className="n">{i + 1}</span>
          <h3>{s.title}</h3>
          <p>{s.body}</p>
        </li>
      ))}
    </ol>
  );
}
