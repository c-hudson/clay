#!/usr/bin/env node
/**
 * Web Interface Screenshot Capture
 *
 * Captures screenshots of the Clay web interface using Puppeteer.
 * Requires: npm install puppeteer
 *
 * Usage: node capture-web.js <base-url> <output-dir>
 * Example: node capture-web.js http://localhost:9000 docs/images/web
 */

const puppeteer = require('puppeteer');
const path = require('path');
const fs = require('fs');

const DEFAULT_URL = 'http://localhost:9000';
const DEFAULT_OUTPUT = 'docs/images/web';

async function captureScreenshots(baseUrl, outputDir) {
    // Ensure output directory exists
    if (!fs.existsSync(outputDir)) {
        fs.mkdirSync(outputDir, { recursive: true });
    }

    console.log('Launching browser...');
    const browser = await puppeteer.launch({
        headless: 'new',
        args: ['--no-sandbox', '--disable-setuid-sandbox']
    });

    const page = await browser.newPage();

    // Set viewport for consistent screenshots
    await page.setViewport({
        width: 1280,
        height: 800,
        deviceScaleFactor: 1
    });

    try {
        console.log(`Navigating to ${baseUrl}...`);
        await page.goto(baseUrl, { waitUntil: 'networkidle2', timeout: 30000 });

        // Wait for page to fully load
        await page.waitForTimeout(2000);

        // Capture login screen (if not authenticated)
        console.log('Capturing login screen...');
        await page.screenshot({
            path: path.join(outputDir, 'web-login.png'),
            fullPage: false
        });

        // If there's a password field, try to authenticate
        const passwordField = await page.$('input[type="password"]');
        if (passwordField) {
            console.log('Password field found, authentication required.');
            console.log('Set CLAY_WEB_PASSWORD environment variable to authenticate.');

            const password = process.env.CLAY_WEB_PASSWORD;
            if (password) {
                await passwordField.type(password);
                await page.click('button[type="submit"]');
                await page.waitForTimeout(2000);
            }
        }

        // Capture main interface
        console.log('Capturing main interface...');
        await page.screenshot({
            path: path.join(outputDir, 'web-main.png'),
            fullPage: false
        });

        // Try to capture world selector
        const worldsButton = await page.$('text=World Selector');
        if (worldsButton) {
            await worldsButton.click();
            await page.waitForTimeout(500);
            await page.screenshot({
                path: path.join(outputDir, 'web-world-selector.png'),
                fullPage: false
            });
            await page.keyboard.press('Escape');
        }

        // Try to capture hamburger menu
        const menuButton = await page.$('.hamburger, .menu-button, [aria-label="Menu"]');
        if (menuButton) {
            await menuButton.click();
            await page.waitForTimeout(500);
            await page.screenshot({
                path: path.join(outputDir, 'web-menu.png'),
                fullPage: false
            });
            await page.keyboard.press('Escape');
        }

        console.log('Screenshots captured successfully!');

    } catch (error) {
        console.error('Error capturing screenshots:', error.message);
    } finally {
        await browser.close();
    }
}

// Parse command line arguments
const args = process.argv.slice(2);
const baseUrl = args[0] || DEFAULT_URL;
const outputDir = args[1] || DEFAULT_OUTPUT;

console.log('=== Clay Web Interface Screenshot Capture ===');
console.log(`URL: ${baseUrl}`);
console.log(`Output: ${outputDir}`);
console.log('');

captureScreenshots(baseUrl, outputDir)
    .then(() => console.log('Done.'))
    .catch(err => {
        console.error('Fatal error:', err);
        process.exit(1);
    });
