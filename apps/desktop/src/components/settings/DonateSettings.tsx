import { Coffee, ExternalLink, Heart, Landmark, WalletCards } from 'lucide-react';
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
    <section className="settings-section donate-settings" aria-labelledby="donate-settings-title">
      <div className="donate-heading">
        <span className="donate-heading-icon" aria-hidden="true">
          <Heart size={19} />
        </span>
        <div>
          <h3 id="donate-settings-title">Support Sky Auto Player</h3>
          <p className="settings-note">Support ongoing development if you enjoy Sky Auto Player.</p>
        </div>
      </div>

      <div className="donate-provider-grid">
        <div className="donate-provider-card">
          <div className="donate-provider-heading">
            <Coffee size={17} aria-hidden="true" />
            <div>
              <h4>Ko-fi</h4>
              <p>Support the project</p>
            </div>
          </div>
          <a
            className="button donate-external-link"
            href={DONATION_CONFIG.koFiUrl}
            aria-label="Open Ko-fi in browser"
            target="_blank"
            rel="noopener noreferrer"
            onClick={(event) => openSupportLink(event, DONATION_CONFIG.koFiUrl)}
          >
            Open <ExternalLink size={14} aria-hidden="true" />
          </a>
        </div>
        <div className="donate-provider-card">
          <div className="donate-provider-heading">
            <WalletCards size={17} aria-hidden="true" />
            <div>
              <h4>PayPal</h4>
              <p>International support</p>
            </div>
          </div>
          <a
            className="button donate-external-link"
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

      <div className="donate-bank-card">
        <div className="donate-provider-heading">
          <Landmark size={17} aria-hidden="true" />
          <h4>Bank transfer · VietQR</h4>
        </div>
        <div className="donate-bank-content">
          <div className="donate-qr-frame">
            <img
              src={DONATION_CONFIG.vietQrImagePath}
              alt="VietQR bank transfer code"
              width={200}
              height={200}
            />
          </div>

          <div className="donate-bank-information">
            <p className="donate-bank-guidance">
              Scan with your phone’s banking app. Verify the recipient before confirming.
            </p>
            <dl className="donate-bank-details">
              <div>
                <dt>Bank</dt>
                <dd>{DONATION_CONFIG.bankName}</dd>
              </div>
              <div>
                <dt>Account holder</dt>
                <dd>{DONATION_CONFIG.accountHolder}</dd>
              </div>
              <div className="donate-bank-reference-row">
                <dt>Account number</dt>
                <dd className="donate-bank-account">{DONATION_CONFIG.accountNumber}</dd>
              </div>
              <div className="donate-bank-reference-row">
                <dt>Transfer note</dt>
                <dd>{DONATION_CONFIG.transferMemo}</dd>
              </div>
            </dl>
          </div>
        </div>
      </div>

      {notice && (
        <p
          className={notice.kind === 'error' ? 'inline-error' : 'settings-note'}
          role={notice.kind === 'error' ? 'alert' : 'status'}
        >
          {notice.text}
        </p>
      )}
      <p className="settings-note donate-disclaimer">
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
