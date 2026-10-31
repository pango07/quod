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
    "How is this different from a cache?",
    "A cache restores what you told it to. Quod decides whether a step needs to run at all, based on everything the step read, and replays the whole result when nothing changed. There are no hand-written keys to get wrong.",
  ],
  [
    "What if a step depends on something Quod can't see?",
    "The network, the clock and other machines can't be hashed. A step that reaches the network never replays. Audits re-run a share of replays to catch anything else, and quarantine a step the moment it diverges.",
  ],
  [
    "Which languages work?",
    "Quod watches the filesystem, so it doesn't need a plugin per language. We've tested Go, Node and Python end to end, and we'll add more as they pass the same tests.",
  ],
  [
    "Where does it run?",
    "On your own Linux machines, as a self-hosted GitHub Actions runner. Your code and secrets stay on your hardware.",
  ],
  [
    "When can I use it?",
    "We're onboarding teams from the waitlist in small groups. Join and we'll reach out when there's a spot.",
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
            <a href="#proof">Proof</a>
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
            <h1>Your CI already did this.</h1>
            <p className="lede">
              Quod replays any step whose inputs haven&rsquo;t changed, and shows you the proof. No config. No rewrite.
            </p>
            <JoinForm source="hero" />
          </div>
          <CiJob />
        </section>

        <section className="sec" id="try">
          <div className="sec-head">
            <h2>Try it in your browser.</h2>
            <p className="lede">
              A real shell with a small Go project. Watch it run, or take over. Edit the README and Quod replays. Edit
              the code and it runs, then tells you exactly which file changed.
            </p>
          </div>
          <Terminal id="term-try" />
        </section>

        <section className="sec" id="how">
          <div className="sec-head">
            <h2>It watches what your steps actually read.</h2>
          </div>
          <Flow />
        </section>

        <section className="sec split">
          <div className="sec-head">
            <h2>Every skip comes with a receipt.</h2>
            <p className="lede">
              A replay is never a black box. Each one tells you which run proved it, what matched and how much time it
              gave back.
            </p>
          </div>
          <Receipt />
        </section>

        <section className="sec" id="proof">
          <div className="sec-head">
            <h2>We tried to break it.</h2>
            <p className="lede">
              A skip is only worth taking if it&rsquo;s safe. So we changed every input Quod recorded, one at a time,
              and checked that each change forced a real run.
            </p>
          </div>
          <div className="proof">
            <div>
              <span className="big">0</span>
              <p>false replays across 29,879 mutated inputs on GitHub&rsquo;s own CLI</p>
            </div>
            <div>
              <span className="big">0.25s</span>
              <p>to replay a 17.0s test step, with byte-identical output</p>
            </div>
            <div>
              <span className="big">3</span>
              <p>ecosystems tested end to end: Go, Node and Python</p>
            </div>
            <div>
              <span className="big">1 line</span>
              <p>in your workflow to run on Quod</p>
            </div>
          </div>
          <p className="foot-note">
            Measured on cli/cli, luxon and starlette in our test harness. The full method and logs ship with the public
            benchmark.
          </p>
        </section>

        <section className="sec split">
          <div className="sec-head">
            <h2>Trust is earned, one step at a time.</h2>
            <dl className="trust">
              <div>
                <dt>Observe first</dt>
                <dd>
                  On day one Quod runs everything and shows you what it would have skipped. Nothing replays until you turn
                  it on.
                </dd>
              </div>
              <div>
                <dt>Proof before replay</dt>
                <dd>
                  A step replays only after it produces the same result from the same inputs more than once. Failures and
                  timeouts never prove themselves.
                </dd>
              </div>
              <div>
                <dt>Audits never stop</dt>
                <dd>
                  A share of replays re-run in full and are compared. One divergence quarantines that step until it proves
                  itself again.
                </dd>
              </div>
            </dl>
          </div>
          <TrustMeter />
        </section>

        <section className="sec split">
          <div className="sec-head">
            <h2>One line in your workflow.</h2>
            <p className="lede">
              Quod runs your existing GitHub Actions jobs on your own Linux machines. Your YAML, your logs and your checks
              stay in GitHub.
            </p>
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
          <p className="lede">Join the waitlist and we&rsquo;ll reach out when there&rsquo;s a spot for your team.</p>
          <JoinForm source="footer" />
        </section>
      </main>

      <footer className="footer">
        <Image src="/quod-logo.png" alt="Quod" width={1280} height={340} className="logo" />
        <span>&copy; 2026 Quod</span>
      </footer>
    </>
  );
}
