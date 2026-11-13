import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { DonateSettings } from './DonateSettings';
import { DONATION_CONFIG } from './donationConfig';

afterEach(cleanup);

describe('DonateSettings', () => {
  it('shows Ko-fi, PayPal and the offline MBBank VietQR without outdated wallet data', () => {
    render(<DonateSettings />);
    expect(screen.getByRole('region', { name: 'Support Sky Auto Player' })).toBeInTheDocument();
    expect(document.querySelector('.donate-heading-icon svg')).toHaveClass('lucide-hand-heart');
    expect(screen.getByRole('link', { name: 'Open Ko-fi in browser' })).toHaveAttribute(
      'href',
      DONATION_CONFIG.koFiUrl,
    );
    expect(screen.getByRole('link', { name: 'Open PayPal in browser' })).toHaveAttribute(
      'href',
      DONATION_CONFIG.paypalUrl,
    );
    expect(screen.getByRole('link', { name: /star on GitHub/ })).toHaveAttribute(
      'href',
      DONATION_CONFIG.githubUrl,
    );

    expect(
      screen
        .getByRole('heading', { name: 'PayPal' })
        .closest('.donate-provider-heading')
        ?.querySelector('svg'),
    ).toHaveClass('lucide-wallet-cards');

    expect(screen.getByRole('heading', { name: 'Bank transfer · VietQR' })).toBeInTheDocument();
    expect(screen.getAllByText(DONATION_CONFIG.bankName)).toHaveLength(1);
    expect(screen.getByText(DONATION_CONFIG.bankName)).toBeInTheDocument();
    expect(screen.getByText(DONATION_CONFIG.accountHolder)).toBeInTheDocument();
    expect(screen.getByText(DONATION_CONFIG.accountNumber)).toBeInTheDocument();
    expect(screen.getByText(DONATION_CONFIG.transferMemo)).toBeInTheDocument();
    expect(screen.queryByText(/No fixed amount|· VND/)).not.toBeInTheDocument();
    expect(document.querySelector('.donate-qr-card')).toBeNull();
    const guidance = screen.getByText(
      'Scan with your phone’s banking app. Verify the recipient before confirming.',
    );
    expect(guidance).toHaveClass('donate-bank-guidance');
    expect(guidance.parentElement).toHaveClass('donate-bank-information');
    expect(document.querySelector('.donate-bank-card > .settings-note')).toBeNull();
    expect(screen.getByRole('img', { name: 'VietQR bank transfer code' })).toHaveAttribute(
      'src',
      DONATION_CONFIG.vietQrImagePath,
    );
    const referenceRows = document.querySelectorAll('.donate-bank-reference-row');
    expect(referenceRows).toHaveLength(2);
    expect(referenceRows[0]).toContainElement(screen.getByText(DONATION_CONFIG.accountNumber));
    expect(referenceRows[1]).toContainElement(screen.getByText(DONATION_CONFIG.transferMemo));
    expect(screen.queryByRole('button', { name: 'Copy account' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Copy note' })).toBeNull();
    const githubLink = screen.getByRole('link', { name: /star on GitHub/ });
    expect(githubLink.querySelector('span')).toHaveTextContent('star on GitHub');
    expect(githubLink.querySelector('svg')).toHaveClass('lucide-external-link');
    expect(screen.queryByText(/MoMo|PSP2603710800000415|Techcombank/i)).not.toBeInTheDocument();
  });

  it('keeps manual transfer details readable without presenting desktop-only copy actions', () => {
    const { container } = render(<DonateSettings />);
    const details = container.querySelector('.donate-bank-details');
    expect(details).not.toBeNull();
    expect(details).toHaveTextContent(DONATION_CONFIG.accountHolder);
    expect(details).toHaveTextContent(DONATION_CONFIG.accountNumber);
    expect(details).toHaveTextContent(DONATION_CONFIG.transferMemo);
    expect(container.querySelector('.donate-copy-button')).toBeNull();
    expect(screen.getByRole('img', { name: 'VietQR bank transfer code' })).toBeInTheDocument();
  });
});
