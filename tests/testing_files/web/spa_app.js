/**
 * Phase 4 Test: Hybrid Web Ecosystem
 * This JavaScript file tests cross-language references (JS → HTML IDs, JS → CSS classes)
 */

// DOM element references - Phase 4 cross-language linking
const appContainer = document.getElementById('app-container');
const mainHeader = document.getElementById('main-header');
const contentArea = document.getElementById('content-area');
const toggleBtn = document.getElementById('toggle-btn');
const loadBtn = document.getElementById('load-btn');
const statusText = document.getElementById('status-text');
const dashboard = document.getElementById('dashboard');
const sidebar = document.getElementById('sidebar');
const navList = document.getElementById('nav-list');
const footer = document.getElementById('footer');

// Alternative selector using querySelector
const headerNav = document.querySelector('#main-header');
const contentSection = document.querySelector('main#content-area');

/**
 * Initialize the application
 */
function initializeApp() {
    // Add event listeners
    toggleBtn.addEventListener('click', handleToggleTheme);
    loadBtn.addEventListener('click', handleLoadData);
    
    // Update status
    statusText.textContent = 'Status: Initialized';
}

/**
 * Handle theme toggle
 * References CSS classes for styling
 */
function handleToggleTheme() {
    const isDark = appContainer.classList.contains('dark-theme');
    
    if (isDark) {
        appContainer.classList.remove('dark-theme');
        appContainer.classList.add('light-theme');
        toggleBtn.classList.remove('btn-dark');
        toggleBtn.classList.add('btn-primary');
    } else {
        appContainer.classList.add('dark-theme');
        appContainer.classList.remove('light-theme');
        toggleBtn.classList.add('btn-dark');
        toggleBtn.classList.remove('btn-primary');
    }
    
    // Update status message
    updateStatusMessage('Theme toggled');
}

/**
 * Handle data loading
 * References CSS classes and HTML elements
 */
function handleLoadData() {
    // Show loading state using CSS classes
    dashboard.classList.add('loading');
    loadBtn.classList.add('btn-disabled');
    statusText.textContent = 'Status: Loading...';
    
    // Simulate async operation
    setTimeout(() => {
        // Restore normal state
        dashboard.classList.remove('loading');
        loadBtn.classList.remove('btn-disabled');
        statusText.textContent = 'Status: Data loaded';
        
        // Populate content
        updateContentArea();
    }, 1000);
}

/**
 * Update content area with data
 */
function updateContentArea() {
    const contentItems = contentArea.querySelectorAll('.content-item');
    contentItems.forEach((item) => {
        item.classList.add('active');
    });
}

/**
 * Update the status message
 */
function updateStatusMessage(message) {
    const element = document.getElementById('status-text');
    if (element) {
        element.textContent = `Status: ${message}`;
        element.classList.add('status-updated');
    }
}

/**
 * Setup navigation
 * References HTML id and CSS classes
 */
function setupNavigation() {
    const navItems = document.getElementById('nav-list');
    if (navItems) {
        const links = navItems.querySelectorAll('a');
        links.forEach((link) => {
            link.addEventListener('click', (e) => {
                e.preventDefault();
                const href = link.getAttribute('href');
                navigateToSection(href);
            });
            // Add active styling via CSS class
            link.classList.add('nav-link-active');
        });
    }
}

/**
 * Navigate to a section
 */
function navigateToSection(sectionId) {
    const section = document.querySelector(sectionId);
    if (section) {
        section.classList.add('visible');
        section.classList.remove('hidden');
    }
}

/**
 * Initialize when DOM is ready
 */
if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', initializeApp);
} else {
    initializeApp();
}

// Setup navigation after a short delay
setTimeout(setupNavigation, 500);
