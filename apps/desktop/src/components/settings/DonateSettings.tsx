import { Coffee, ExternalLink, HandHeart, Landmark, WalletCards } from 'lucide-react';
import { useState, type MouseEvent } from 'react';
import { openExternalUrl } from '../../platform/externalLinks';
import { isTauriRuntime } from '../../platform/runtime';
import { DONATION_CONFIG } from './donationConfig';

type Notice = { kind: 'success' | 'error'; text: string };

export function DonateSettings() {
  const [notice, setNotice] = useState<Notice | null>(null);

  function openSupportLink(event: MouseEvent<HTMLAnchorElement>, url: string) {
    if (!isTauriRuntime()) return;
    event.preventDefault();
    setNotice(null);
    void (async () => {
      try {
        await openExternalUrl(url);
      } catch {
        setNotice({
          kind: 'error',
          text: `Could not open the browser. Visit ${url} manually.`,
        });
      }
    })();
  }

  return (
    <section
      data-settings-category="donate"
      data-slot="settings-category-content"
      className="grid content-start gap-3 py-4"
      aria-labelledby="donate-settings-title"
    >
      <div data-slot="donate-heading" className="flex items-center gap-3">
        <span
          data-slot="donate-heading-icon"
          className="grid size-9 flex-none place-items-center rounded-control border border-primary/30 bg-selection text-accent-ink"
          aria-hidden="true"
        >
          <HandHeart size={19} />
        </span>
        <div className="min-w-0">
          <h3 className="m-0 text-[15px] font-semibold text-foreground" id="donate-settings-title">
            Support Sky Auto Player
          </h3>
          <p className="m-0 mt-1 text-xs leading-relaxed text-text-tertiary">
            Support ongoing development if you enjoy Sky Auto Player.
          </p>
        </div>
      </div>

      <div className="grid grid-cols-2 gap-3 max-[700px]:grid-cols-1">
        <div
          data-slot="donate-provider-card"
          className="flex min-w-0 items-center justify-between gap-3 rounded-pane border border-border bg-surface p-3"
        >
          <div data-slot="donate-provider-heading" className="flex min-w-0 items-center gap-2.5">
            <Coffee className="text-text-secondary" size={17} aria-hidden="true" />
            <div className="min-w-0">
              <h4 className="m-0 text-[13px] font-semibold text-foreground">Ko-fi</h4>
              <p className="m-0 text-xs text-text-tertiary">Support the project</p>
            </div>
          </div>
          <a
            data-slot="donate-external-link"
            className="inline-flex h-8 flex-none items-center gap-1.5 rounded-control border border-border bg-card px-2.5 text-xs font-medium text-text-secondary hover:bg-control-hover hover:text-foreground"
            href={DONATION_CONFIG.koFiUrl}
            aria-label="Open Ko-fi in browser"
            target="_blank"
            rel="noopener noreferrer"
            onClick={(event) => openSupportLink(event, DONATION_CONFIG.koFiUrl)}
          >
            Open <ExternalLink size={14} aria-hidden="true" />
          </a>
        </div>
        <div
          data-slot="donate-provider-card"
          className="flex min-w-0 items-center justify-between gap-3 rounded-pane border border-border bg-surface p-3"
        >
          <div data-slot="donate-provider-heading" className="flex min-w-0 items-center gap-2.5">
            <WalletCards className="text-text-secondary" size={17} aria-hidden="true" />
            <div className="min-w-0">
              <h4 className="m-0 text-[13px] font-semibold text-foreground">PayPal</h4>
              <p className="m-0 text-xs text-text-tertiary">International support</p>
            </div>
          </div>
          <a
            data-slot="donate-external-link"
            className="inline-flex h-8 flex-none items-center gap-1.5 rounded-control border border-border bg-card px-2.5 text-xs font-medium text-text-secondary hover:bg-control-hover hover:text-foreground"
            href={DONATION_CONFIG.paypalUrl}
            aria-label="Open PayPal in browser"
            target="_blank"
            rel="noopener noreferrer"
            onClick={(event) => openSupportLink(event, DONATION_CONFIG.paypalUrl)}
          >
            Open <ExternalLink size={14} aria-hidden="true" />
          </a>
        </div>
      </div>

      <div
        data-slot="donate-bank-card"
        className="grid gap-3 rounded-pane border border-border bg-surface p-3"
      >
        <div className="flex items-center gap-2.5">
          <Landmark size={17} aria-hidden="true" />
          <h4 className="m-0 text-[13px] font-semibold text-foreground">Bank transfer · VietQR</h4>
        </div>
        <div className="grid min-w-0 grid-cols-[160px_minmax(0,1fr)] items-start gap-4 max-[700px]:grid-cols-1">
          <div
            data-slot="donate-qr-frame"
            className="grid size-40 place-items-center rounded-control border border-border bg-white p-2"
          >
            <img
              src={DONATION_CONFIG.vietQrImagePath}
              alt="VietQR bank transfer code"
              className="size-full object-contain"
              width={200}
              height={200}
            />
          </div>

          <div data-slot="donate-bank-information" className="min-w-0">
            <p
              data-slot="donate-bank-guidance"
              className="m-0 text-xs leading-relaxed text-text-secondary"
            >
              Scan with your phone’s banking app. Verify the recipient before confirming.
            </p>
            <dl data-slot="donate-bank-details" className="mb-0 mt-2 grid gap-0">
              <div className="grid grid-cols-[110px_minmax(0,1fr)] items-baseline gap-3 border-b border-border py-1 last:border-b-0">
                <dt className="text-xs text-text-tertiary">Bank</dt>
                <dd className="m-0 min-w-0 break-words text-xs text-foreground">
                  {DONATION_CONFIG.bankName}
                </dd>
              </div>
              <div className="grid grid-cols-[110px_minmax(0,1fr)] items-baseline gap-3 border-b border-border py-1 last:border-b-0">
                <dt className="text-xs text-text-tertiary">Account holder</dt>
                <dd className="m-0 min-w-0 break-words text-xs text-foreground">
                  {DONATION_CONFIG.accountHolder}
                </dd>
              </div>
              <div
                data-slot="donate-bank-reference-row"
                className="grid grid-cols-[110px_minmax(0,1fr)] items-baseline gap-3 border-b border-border py-1 last:border-b-0"
              >
                <dt className="text-xs text-text-tertiary">Account number</dt>
                <dd className="m-0 min-w-0 break-words font-mono text-xs font-semibold tabular-nums text-foreground">
                  {DONATION_CONFIG.accountNumber}
                </dd>
              </div>
              <div
                data-slot="donate-bank-reference-row"
                className="grid grid-cols-[110px_minmax(0,1fr)] items-baseline gap-3 py-1"
              >
                <dt className="text-xs text-text-tertiary">Transfer note</dt>
                <dd className="m-0 min-w-0 break-words text-xs text-foreground">
                  {DONATION_CONFIG.transferMemo}
                </dd>
              </div>
            </dl>
          </div>
        </div>
      </div>

      {notice && (
        <p
          className={`m-0 text-xs ${notice.kind === 'error' ? 'text-danger' : 'text-text-tertiary'}`}
          role={notice.kind === 'error' ? 'alert' : 'status'}
        >
          {notice.text}
        </p>
      )}
      <p className="m-0 text-xs leading-relaxed text-text-tertiary [&_a]:inline-flex [&_a]:items-center [&_a]:gap-1 [&_a]:font-medium [&_a]:text-accent-ink [&_a]:underline-offset-2 hover:[&_a]:underline focus-within:[&_a]:outline-2 focus-within:[&_a]:outline-ring">
        Donations are optional. Sky Auto Player does not process or verify payments. You can also{' '}
        <a
          href={DONATION_CONFIG.githubUrl}
          target="_blank"
          rel="noopener noreferrer"
          onClick={(event) => openSupportLink(event, DONATION_CONFIG.githubUrl)}
        >
          <span>star on GitHub</span>
          <ExternalLink size={12} aria-hidden="true" />
        </a>
        .
      </p>
    </section>
  );
}
