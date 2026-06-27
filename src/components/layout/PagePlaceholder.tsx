type PagePlaceholderProps = {
  title: string;
  message?: string;
};

export function PagePlaceholder({ title, message }: PagePlaceholderProps) {
  return (
    <div className="flex flex-col items-center justify-center h-full text-center py-20">
      <h2 className="text-2xl font-semibold text-gray-800 dark:text-gray-200 mb-3">{title}</h2>
      <p className="text-gray-500 dark:text-gray-400 max-w-md">
        {message || `${title}功能将在后续子任务中实现。`}
      </p>
    </div>
  );
}
