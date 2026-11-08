import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { DonateSettings } from './DonateSettings';
import { DONATION_CONFIG } from './donationConfig';

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('DonateSettings', () => {
  it('shows Ko-fi, PayPal, and the matching MoMo receive QR', () => {
    render(<DonateSettings />);
    expect(screen.getByRole('region', { name: 'Support Sky Auto Player' })).toBeInTheDocument();
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
    expect(screen.getByText(DONATION_CONFIG.momoAccountHolder)).toBeInTheDocument();
    expect(screen.getByText(DONATION_CONFIG.momoPhoneNumber)).toBeInTheDocument();
    expect(screen.getByText(DONATION_CONFIG.momoReceivingId)).toBeInTheDocument();
    expect(screen.getByText(DONATION_CONFIG.transferMemo)).toBeInTheDocument();
    expect(screen.queryByText(/Techcombank/i)).not.toBeInTheDocument();
    expect(
      screen.getByRole('img', { name: /MoMo receive QR for Nguyen Duc Hoa/ }),
    ).toHaveAttribute('src', DONATION_CONFIG.momoQrImagePath);
    expect(
      screen.queryByRole('group', { name: 'Official MoMo QR not yet configured' }),
    ).not.toBeInTheDocument();
    expect(screen.getByText(/Scan the QR with a supported banking app/)).toBeInTheDocument();
  });

  it('copies MoMo phone, PSP receiving account and transfer note', async () => {
    const previousClipboard = Object.getOwnPropertyDescriptor(navigator, 'clipboard');
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText },
    });
    try {
      render(<DonateSettings />);
      fireEvent.click(screen.getByRole('button', { name: 'Copy phone' }));
      await waitFor(() =>
        expect(writeText).toHaveBeenCalledWith(DONATION_CONFIG.momoPhoneNumber),
      );
      expect(screen.getByRole('status')).toHaveTextContent('MoMo phone number copied.');
      fireEvent.click(screen.getByRole('button', { name: 'Copy account' }));
      await waitFor(() =>
        expect(writeText).toHaveBeenCalledWith(DONATION_CONFIG.momoReceivingId),
      );
      expect(screen.getByRole('status')).toHaveTextContent('MoMo account copied.');
      fireEvent.click(screen.getByRole('button', { name: 'Copy note' }));
      await waitFor(() => expect(writeText).toHaveBeenCalledWith(DONATION_CONFIG.transferMemo));
      expect(screen.getByRole('status')).toHaveTextContent('Transfer note copied.');
    } finally {
      if (previousClipboard) {
        Object.defineProperty(navigator, 'clipboard', previousClipboard);
      } else {
        Reflect.deleteProperty(navigator, 'clipboard');
      }
    }
  });

  it('keeps the wallet details selectable when clipboard is unavailable', async () => {
    const previousClipboard = Object.getOwnPropertyDescriptor(navigator, 'clipboard');
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText: vi.fn().mockRejectedValue(new Error('denied')) },
    });
    try {
      render(<DonateSettings />);
      fireEvent.click(screen.getByRole('button', { name: 'Copy phone' }));
      expect(await screen.findByRole('alert')).toHaveTextContent('Could not copy');
      expect(screen.getByText(DONATION_CONFIG.momoPhoneNumber)).toBeInTheDocument();
    } finally {
      if (previousClipboard) {
        Object.defineProperty(navigator, 'clipboard', previousClipboard);
      } else {
        Reflect.deleteProperty(navigator, 'clipboard');
      }
    }
  });
});
