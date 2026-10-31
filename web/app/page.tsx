import Image from "next/image";
import { CiJob } from "@/components/ci-job";
import { Flow } from "@/components/flow";
import { JoinForm } from "@/components/join-form";
import { Receipt } from "@/components/receipt";
import { Terminal } from "@/components/terminal";
import { TrustMeter } from "@/components/trust-meter";
import { WorkflowDiff } from "@/components/workflow-diff";

const FAQ: [string, string][] = [
  [
    "Is this just caching?",
    "A cache saves files you pick. Quod skips whole steps, like your test run, and it decides by looking at every file the step actually read. You don't configure anything.",
  ],
  [
    "What if it reuses a result it shouldn't?",
    "It only reuses a result when every file the step read is exactly the same. Steps that use the network are never reused, and Quod keeps re-running a few reused steps to double-check.",
  ],
  [
    "Which languages?",
    "Go, Node and Python today. Quod watches files, not code, so more are coming.",
  ],
  [
    "Where does it run?",
    "On your own Linux machines, with your existing GitHub Actions workflows.",
  ],
];

export default function Home() {
  return (
    <>
      <header className="nav-bar">
        <div className="nav">
          <a href="#top" aria-label="Quod home">
            <Image src="/quod-logo.png" alt="Quod" width={1280} height={340} priority className="logo" />
          </a>
          <nav aria-label="Main">
            <a href="#how">How it works</a>
            <a href="#try">Try it</a>
            <a href="#faq">FAQ</a>
            <a className="btn small" href="#join">
              Join the waitlist
            </a>
          </nav>
        </div>
      </header>

      <main id="top">
        <section className="hero">
          <div className="hero-txt">
            <h1>Skip the CI work you already did.</h1>
            <p className="lede">
              If nothing a CI step depends on has changed, Quod reuses its last result instead of running it again. Your
              checks finish in seconds, not minutes.
            </p>
            <JoinForm source="hero" />
          </div>
          <CiJob />
        </section>

        <section className="sec" id="how">
          <div className="sec-head">
            <h2>How it works</h2>
          </div>
          <Flow />
        </section>

        <section className="sec" id="try">
          <div className="sec-head">
            <h2>Try it</h2>
            <p className="lede">Change a file, run the tests, and see what Quod reuses.</p>
          </div>
          <Terminal id="term-try" />
        </section>

        <section className="sec split">
          <div className="sec-head">
            <h2>Every skip shows its proof.</h2>
            <p className="lede">You always see which run a result came from and which files matched.</p>
          </div>
          <Receipt />
        </section>

        <section className="sec split" id="safe">
          <div className="sec-head">
            <h2>Safe by default.</h2>
            <ul className="points">
              <li>It starts by only watching. Nothing is skipped until you turn it on.</li>
              <li>A result is reused only after the step has produced it twice.</li>
              <li>It keeps re-running a few skipped steps to make sure.</li>
            </ul>
          </div>
          <TrustMeter />
        </section>

        <section className="sec" id="proof">
          <div className="proof">
            <div>
              <span className="big">0.25s</span>
              <p>instead of 17s for a test step that didn&rsquo;t need to run</p>
            </div>
            <div>
              <span className="big">0</span>
              <p>wrong skips when we changed each of 29,879 files one at a time</p>
            </div>
            <div>
              <span className="big">3</span>
              <p>languages tested end to end: Go, Node and Python</p>
            </div>
          </div>
        </section>

        <section className="sec split">
          <div className="sec-head">
            <h2>One line to set up.</h2>
            <p className="lede">Point your GitHub Actions job at Quod. Everything else stays the same.</p>
          </div>
          <WorkflowDiff />
        </section>

        <section className="sec" id="faq">
          <div className="sec-head">
            <h2>Questions</h2>
          </div>
          <div className="faq">
            {FAQ.map(([q, a]) => (
              <details key={q}>
                <summary>{q}</summary>
                <p>{a}</p>
              </details>
            ))}
          </div>
        </section>

        <section className="cta" id="join">
          <h2>Push. Prove. Done.</h2>
          <p className="lede">We&rsquo;re letting teams in a few at a time. Join the waitlist.</p>
          <JoinForm source="footer" />
        </section>
      </main>

      <footer className="footer">
        <Image src="/quod-logo.png" alt="Quod" width={1280} height={340} className="logo" />
        <span>Quod (kwod), from Q.E.D. &copy; 2026</span>
      </footer>
    </>
  );
}
