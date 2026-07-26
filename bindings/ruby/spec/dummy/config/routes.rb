# frozen_string_literal: true

Rails.application.routes.draw do
  %w[show download with_layout as_html with_stylesheet bad_option].each do |action|
    get "/invoices/#{action}", to: "invoices##{action}"
  end
end
